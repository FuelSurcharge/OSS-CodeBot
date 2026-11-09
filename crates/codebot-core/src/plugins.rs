//! Pure WebAssembly extensions: no WASI, imports, ambient files, subprocesses or
//! network access. A guest can return data and one separately reviewed proposal.
use crate::ToolCall;
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashSet,
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
const MAX_MODULE: usize = 1024 * 1024;
const MAX_INPUT: usize = 256 * 1024;
const MAX_OUTPUT: usize = 1024 * 1024;
const FUEL: u64 = 10_000_000;
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn valid_id(s: &str) -> bool {
    (1..=64).contains(&s.len())
        && s.bytes()
            .next()
            .is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        && !matches!(
            s,
            "con"
                | "prn"
                | "aux"
                | "nul"
                | "com1"
                | "com2"
                | "com3"
                | "com4"
                | "com5"
                | "com6"
                | "com7"
                | "com8"
                | "com9"
                | "lpt1"
                | "lpt2"
                | "lpt3"
                | "lpt4"
                | "lpt5"
                | "lpt6"
                | "lpt7"
                | "lpt8"
                | "lpt9"
        )
        && s.bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_')
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PluginPermission {
    ReadSelectedFiles,
    ProposeChanges,
    ProposeCommand,
    Network,
}
impl PluginPermission {
    pub fn label(self) -> &'static str {
        match self {
            Self::ReadSelectedFiles => "選択して渡したファイル内容を読む",
            Self::ProposeChanges => "ファイル変更を提案する（適用には個別の確認）",
            Self::ProposeCommand => {
                "コマンドを提案する（実行には個別の確認。ファイル操作・外部通信が可能）"
            }
            Self::Network => "直接のネットワーク通信（未対応・許可できません）",
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    pub manifest_version: u32,
    pub api_version: u32,
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub module: String,
    pub sha256: String,
    pub permissions: Vec<PluginPermission>,
}
impl PluginManifest {
    fn validate(&self) -> Result<(), String> {
        if self.manifest_version != 1
            || self.api_version != 1
            || !valid_id(&self.id)
            || self.module != "plugin.wasm"
            || self.name.trim().is_empty()
            || self.name.chars().count() > 100
            || self.name.chars().any(char::is_control)
            || self.version.is_empty()
            || self.version.len() > 64
            || !self
                .version
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b".-_+".contains(&b))
            || self.description.chars().count() > 2000
            || self
                .description
                .chars()
                .any(|c| c.is_control() && !matches!(c, '\n' | '\t'))
            || self.sha256.len() != 64
            || !self.sha256.bytes().all(|b| b.is_ascii_hexdigit())
            || self.permissions.len() > 4
            || self.permissions.iter().collect::<HashSet<_>>().len() != self.permissions.len()
        {
            return Err("拡張のmanifest・APIバージョン・権限・SHA-256が不正です".into());
        }
        Ok(())
    }
    pub fn supported(&self) -> bool {
        !self.permissions.contains(&PluginPermission::Network)
    }
    fn review_hash(&self) -> String {
        digest(&serde_json::to_vec(self).unwrap())
    }
}
pub struct PreparedPluginInstall {
    manifest: PluginManifest,
    module: Vec<u8>,
}
impl PreparedPluginInstall {
    pub fn manifest(&self) -> &PluginManifest {
        &self.manifest
    }
    pub fn module_bytes(&self) -> usize {
        self.module.len()
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    version: u32,
    enabled: bool,
    reviewed_manifest_sha256: String,
}
#[derive(Clone, Serialize)]
pub struct PluginEntry {
    pub id: String,
    pub name: String,
    pub version: String,
    pub permissions: Vec<PluginPermission>,
    pub enabled: bool,
    pub problem: Option<String>,
}
#[derive(Serialize)]
pub struct PluginRun {
    pub plugin_id: String,
    pub text: String,
    pub proposal: Option<ToolCall>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GuestOutput {
    text: String,
    proposal: Option<GuestProposal>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GuestProposal {
    name: String,
    arguments: serde_json::Value,
}
pub fn default_plugins_dir() -> Option<PathBuf> {
    crate::default_config_path().map(|p| p.with_file_name("plugins"))
}
fn read_file(path: &Path, max: usize) -> Result<Vec<u8>, String> {
    let metadata = fs::symlink_metadata(path).map_err(|_| "拡張のファイルを読み込めません")?;
    if !metadata.is_file() || metadata.file_type().is_symlink() || metadata.len() > max as u64 {
        return Err("拡張のファイル形式・サイズ・リンクを確認してください".into());
    }
    let mut bytes = Vec::new();
    fs::File::open(path)
        .map_err(|_| "拡張のファイルを開けません")?
        .take((max + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "拡張のファイルを読み込めません")?;
    if bytes.len() > max {
        return Err("拡張のファイルが上限を超えています".into());
    }
    Ok(bytes)
}
fn engine() -> wasmi::Engine {
    let mut config = wasmi::Config::default();
    config.consume_fuel(true);
    config.compilation_mode(wasmi::CompilationMode::Eager);
    wasmi::Engine::new(&config)
}
fn module(engine: &wasmi::Engine, bytes: &[u8]) -> Result<wasmi::Module, String> {
    if !bytes.starts_with(b"\0asm\x01\0\0\0") || bytes.len() > MAX_MODULE {
        return Err("1 MiB以下のWebAssembly v1モジュールを指定してください".into());
    }
    let module =
        wasmi::Module::new(engine, bytes).map_err(|_| "拡張のWebAssemblyを検証できません")?;
    if module.imports().next().is_some() {
        return Err("この拡張はWASI・外部importを要求しています。直接のファイル・コマンド・通信機能は許可しません".into());
    }
    if !matches!(
        module.get_export("memory"),
        Some(wasmi::ExternType::Memory(_))
    ) {
        return Err("拡張API v1のmemory exportがありません".into());
    }
    for (name, arity) in [
        ("codebot_alloc", 1),
        ("codebot_run", 2),
        ("codebot_result_len", 0),
    ] {
        let valid = matches!(module.get_export(name),Some(wasmi::ExternType::Func(ref ty)) if ty.params()==vec![wasmi::ValType::I32;arity].as_slice() && ty.results()==[wasmi::ValType::I32]);
        if !valid {
            return Err("拡張API v1の関数export・引数・戻り値が不正です".into());
        }
    }
    Ok(module)
}
pub fn prepare_plugin_install(manifest_path: &Path) -> Result<PreparedPluginInstall, String> {
    let raw = read_file(manifest_path, 64 * 1024)?;
    let manifest: PluginManifest =
        serde_json::from_slice(&raw).map_err(|_| "拡張のmanifest JSON形式が不正です")?;
    manifest.validate()?;
    let module_bytes = read_file(&manifest_path.with_file_name("plugin.wasm"), MAX_MODULE)?;
    if !manifest.sha256.eq_ignore_ascii_case(&digest(&module_bytes)) {
        return Err("拡張モジュールのSHA-256がmanifestと一致しません".into());
    }
    module(&engine(), &module_bytes)?;
    Ok(PreparedPluginInstall {
        manifest,
        module: module_bytes,
    })
}
struct Lease {
    _file: fs::File,
}
fn store(root: &Path, exclusive: bool) -> Result<(PathBuf, Lease), String> {
    if !root.is_absolute() {
        return Err("拡張の保存先は絶対パスで指定してください".into());
    }
    fs::create_dir_all(root).map_err(|_| "拡張の保存先を作成できません")?;
    let root = fs::canonicalize(root).map_err(|_| "拡張の保存先を確認できません")?;
    let path = root.join(".plugins.lock");
    if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("拡張のロックがリンクです".into());
    }
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|_| "拡張の保存先をロックできません")?;
    if exclusive {
        FileExt::try_lock_exclusive(&file)
    } else {
        FileExt::try_lock_shared(&file)
    }
    .map_err(|_| "拡張を別の処理で使用中です。処理後に再実行してください")?;
    Ok((root, Lease { _file: file }))
}
fn location(root: &Path, id: &str) -> Result<PathBuf, String> {
    if !valid_id(id) {
        return Err("拡張IDが不正です".into());
    }
    let path = root.join(id);
    let meta = fs::symlink_metadata(&path).map_err(|_| "拡張が導入されていません")?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err("拡張の保存先が通常のフォルダではありません".into());
    }
    Ok(path)
}
fn installed(path: &Path, id: &str) -> Result<(PluginManifest, State, Vec<u8>), String> {
    let manifest: PluginManifest =
        serde_json::from_slice(&read_file(&path.join("manifest.json"), 64 * 1024)?)
            .map_err(|_| "導入した拡張のmanifestが不正です")?;
    manifest.validate()?;
    let state: State = serde_json::from_slice(&read_file(&path.join("state.json"), 8192)?)
        .map_err(|_| "拡張の有効状態が不正です")?;
    if manifest.id != id
        || state.version != 1
        || state.reviewed_manifest_sha256 != manifest.review_hash()
        || !manifest.supported()
    {
        return Err(
            "導入時に確認した拡張の内容・権限が変更されています。再導入してください".into(),
        );
    }
    let bytes = read_file(&path.join("plugin.wasm"), MAX_MODULE)?;
    if !manifest.sha256.eq_ignore_ascii_case(&digest(&bytes)) {
        return Err("導入した拡張モジュールのSHA-256が一致しません".into());
    }
    Ok((manifest, state, bytes))
}
pub fn install_plugin(root: &Path, plan: &PreparedPluginInstall) -> Result<(), String> {
    if !plan.manifest.supported() {
        return Err("拡張からの直接通信には未対応です。この権限は許可できません".into());
    }
    let (root, _lease) = store(root, true)?;
    let destination = root.join(&plan.manifest.id);
    if destination
        .try_exists()
        .map_err(|_| "導入先を確認できません")?
    {
        return Err("同じIDの拡張が導入済みです。既存の拡張を確認してください".into());
    }
    let count = fs::read_dir(&root)
        .map_err(|_| "拡張の保存先を読み込めません")?
        .take(1001)
        .filter_map(Result::ok)
        .filter(|entry| entry.file_name().to_str().is_some_and(valid_id))
        .count();
    if count >= 200 {
        return Err("拡張の導入は200件までです。使用しない拡張を確認してください".into());
    }
    let temp = tempfile::Builder::new()
        .prefix(".install-")
        .tempdir_in(&root)
        .map_err(|_| "拡張の導入先を作成できません")?;
    let state = State {
        version: 1,
        enabled: false,
        reviewed_manifest_sha256: plan.manifest.review_hash(),
    };
    for (name, bytes) in [
        ("plugin.wasm", plan.module.clone()),
        (
            "manifest.json",
            serde_json::to_vec_pretty(&plan.manifest).unwrap(),
        ),
        ("state.json", serde_json::to_vec(&state).unwrap()),
    ] {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(temp.path().join(name))
            .map_err(|_| "拡張を配置できません")?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|_| "拡張を保存できません")?;
    }
    fs::rename(temp.path(), destination).map_err(|_| "拡張の導入を確定できません")?;
    Ok(())
}
pub fn list_plugins(root: &Path) -> Result<Vec<PluginEntry>, String> {
    let (root, _lease) = store(root, false)?;
    let mut entries = Vec::new();
    for entry in fs::read_dir(&root)
        .map_err(|_| "拡張一覧を読み込めません")?
        .take(1001)
    {
        let entry = entry.map_err(|_| "拡張一覧を読み込めません")?;
        let id = entry
            .file_name()
            .to_str()
            .ok_or("拡張IDの文字が不正です")?
            .to_owned();
        if !valid_id(&id) {
            continue;
        }
        if entries.len() >= 200 {
            return Err("導入した拡張が200件の上限を超えています".into());
        }
        match location(&root, &id).and_then(|p| installed(&p, &id)) {
            Ok((manifest, state, _)) => entries.push(PluginEntry {
                id,
                name: manifest.name,
                version: manifest.version,
                permissions: manifest.permissions,
                enabled: state.enabled,
                problem: None,
            }),
            Err(error) => entries.push(PluginEntry {
                name: id.clone(),
                id,
                version: String::new(),
                permissions: vec![],
                enabled: false,
                problem: Some(error),
            }),
        }
    }
    entries.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(entries)
}
pub fn set_plugin_enabled(root: &Path, id: &str, enabled: bool) -> Result<(), String> {
    let (root, _lease) = store(root, true)?;
    let path = location(&root, id)?;
    let (_, mut state, bytes) = installed(&path, id)?;
    module(&engine(), &bytes)?;
    state.enabled = enabled;
    let mut file =
        tempfile::NamedTempFile::new_in(&path).map_err(|_| "拡張の状態を更新できません")?;
    serde_json::to_writer(file.as_file_mut(), &state).map_err(|_| "拡張の状態を保存できません")?;
    file.as_file_mut()
        .sync_all()
        .map_err(|_| "拡張の状態を保存できません")?;
    file.persist(path.join("state.json"))
        .map_err(|_| "拡張の状態を更新できません")?;
    Ok(())
}
pub struct PreparedPluginRemove {
    root: PathBuf,
    id: String,
    stamp: String,
}
impl PreparedPluginRemove {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn root(&self) -> &Path {
        &self.root
    }
}
fn removal_stamp(path: &Path) -> Result<String, String> {
    let mut hash = Sha256::new();
    let mut names = fs::read_dir(path)
        .map_err(|_| "拡張のファイルを確認できません")?
        .take(5)
        .map(|e| e.map(|e| e.file_name()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|_| "拡張のファイルを確認できません")?;
    names.sort();
    for name in names {
        let s = name.to_str().ok_or("拡張のファイル名が不正です")?;
        let max = match s {
            "plugin.wasm" => MAX_MODULE,
            "manifest.json" => 64 * 1024,
            "state.json" => 8192,
            _ => {
                return Err(
                    "拡張のフォルダに管理外のファイルがあります。退避してから削除してください"
                        .into(),
                )
            }
        };
        hash.update(s.as_bytes());
        hash.update(Sha256::digest(read_file(&path.join(&name), max)?));
    }
    Ok(format!("{:x}", hash.finalize()))
}
pub fn prepare_plugin_remove(root: &Path, id: &str) -> Result<PreparedPluginRemove, String> {
    let (root, _lease) = store(root, true)?;
    let path = location(&root, id)?;
    let stamp = removal_stamp(&path)?;
    Ok(PreparedPluginRemove {
        root,
        id: id.into(),
        stamp,
    })
}
pub fn remove_plugin(plan: &PreparedPluginRemove) -> Result<(), String> {
    let (_root, _lease) = store(&plan.root, true)?;
    let path = location(&plan.root, &plan.id)?;
    if removal_stamp(&path)? != plan.stamp {
        return Err("確認後に拡張が変更されました。削除対象を再確認してください".into());
    }
    fs::remove_dir_all(path)
        .map_err(|_| "拡張の削除が完了していません。一覧を更新してください".into())
}
fn guest_error(error: wasmi::Error) -> String {
    if error.as_trap_code() == Some(wasmi::TrapCode::OutOfFuel) {
        "拡張処理が実行上限に達したため停止しました"
    } else {
        "拡張処理が停止したか、メモリ・入出力・API形式が不正です。アプリのほかの機能は継続できます"
    }
    .into()
}
fn execute(bytes: &[u8], input: &str, cancel: &AtomicBool) -> Result<GuestOutput, String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("拡張の処理を中止しました".into());
    }
    let engine = engine();
    let module = module(&engine, bytes)?;
    let limits = wasmi::StoreLimitsBuilder::new()
        .memory_size(64 * 1024 * 1024)
        .table_elements(10000)
        .instances(1)
        .tables(1)
        .memories(1)
        .trap_on_grow_failure(true)
        .build();
    let mut store = wasmi::Store::new(&engine, limits);
    store.limiter(|s| s);
    store.set_fuel(FUEL).map_err(guest_error)?;
    let linker = wasmi::Linker::<wasmi::StoreLimits>::new(&engine);
    let execution_error = |error| {
        if cancel.load(Ordering::Relaxed) {
            "拡張の処理を中止しました".into()
        } else {
            guest_error(error)
        }
    };
    let instance = linker
        .instantiate_and_start(&mut store, &module)
        .map_err(execution_error)?;
    let memory = instance
        .get_memory(&store, "memory")
        .ok_or("拡張のmemoryがありません")?;
    let alloc = instance
        .get_typed_func::<i32, i32>(&store, "codebot_alloc")
        .map_err(guest_error)?;
    let run = instance
        .get_typed_func::<(i32, i32), i32>(&store, "codebot_run")
        .map_err(guest_error)?;
    let result_len = instance
        .get_typed_func::<(), i32>(&store, "codebot_result_len")
        .map_err(guest_error)?;
    let pointer = alloc
        .call(&mut store, input.len() as i32)
        .map_err(execution_error)?;
    let pointer = usize::try_from(pointer).map_err(|_| "拡張の入力ポインターが不正です")?;
    memory
        .write(&mut store, pointer, input.as_bytes())
        .map_err(|_| "拡張の入力メモリが範囲外です")?;
    if cancel.load(Ordering::Relaxed) {
        return Err("拡張の処理を中止しました".into());
    }
    let output = run
        .call(&mut store, (pointer as i32, input.len() as i32))
        .map_err(execution_error)?;
    let length = result_len.call(&mut store, ()).map_err(execution_error)?;
    if cancel.load(Ordering::Relaxed) {
        return Err("拡張の処理を中止しました。提案は実行していません".into());
    }
    if output < 0 || length < 0 || length as usize > MAX_OUTPUT {
        return Err("拡張の出力サイズ・ポインターが不正です".into());
    }
    let mut data = vec![0; length as usize];
    memory
        .read(&store, output as usize, &mut data)
        .map_err(|_| "拡張の出力メモリが範囲外です")?;
    let output: GuestOutput =
        serde_json::from_slice(&data).map_err(|_| "拡張の出力JSONが不正です")?;
    if output.text.chars().count() > 64000 {
        return Err("拡張の表示テキストが上限を超えています".into());
    }
    Ok(output)
}
pub fn run_plugin(
    root: &Path,
    id: &str,
    input: &str,
    has_selected_files: bool,
    workspace: &Path,
    cancel: &AtomicBool,
) -> Result<PluginRun, String> {
    if cancel.load(Ordering::Relaxed) {
        return Err("拡張の処理を中止しました".into());
    }
    if input.len() > MAX_INPUT || input.chars().count() > 64000 {
        return Err("拡張への入力は64000文字・256 KiB以内にしてください".into());
    }
    let (root, _lease) = store(root, false)?;
    let path = location(&root, id)?;
    let (manifest, state, bytes) = installed(&path, id)?;
    if !state.enabled {
        return Err("この拡張は無効です。権限を確認して有効にしてください".into());
    }
    if has_selected_files
        && !manifest
            .permissions
            .contains(&PluginPermission::ReadSelectedFiles)
    {
        return Err("この拡張には選択ファイルを読む権限がありません。添付を外してください".into());
    }
    let output = execute(&bytes, input, cancel)?;
    let proposal = output
        .proposal
        .map(|p| {
            let permission = match p.name.as_str() {
                "codebot_run_command" => PluginPermission::ProposeCommand,
                "codebot_propose_changes" => PluginPermission::ProposeChanges,
                _ => return Err("拡張が対応していない操作を要求しました".to_owned()),
            };
            if !manifest.permissions.contains(&permission) {
                return Err("拡張が導入時に許可していない操作を要求しました。実行しません".into());
            }
            let call = ToolCall {
                id: format!("plugin-{id}"),
                name: p.name,
                arguments: p.arguments,
            };
            let _ = call.action(workspace)?;
            Ok(call)
        })
        .transpose()?;
    let text = output
        .text
        .chars()
        .map(|c| {
            if c.is_control() && !matches!(c, '\n' | '\t') {
                '�'
            } else {
                c
            }
        })
        .collect();
    Ok(PluginRun {
        plugin_id: id.into(),
        text,
        proposal,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn guest(output: serde_json::Value, body: &str) -> Vec<u8> {
        let raw = serde_json::to_vec(&output).unwrap();
        let escaped = raw.iter().map(|b| format!("\\{b:02x}")).collect::<String>();
        let start = if body == "START_LOOP" {
            "(func $init (loop $forever br $forever)) (start $init)"
        } else {
            ""
        };
        let body = if body == "START_LOOP" {
            "i32.const 65536"
        } else {
            body
        };
        wat::parse_str(format!(
            r#"(module
            {start}
            (memory (export "memory") 2 65536)
            (data (i32.const 65536) "{escaped}")
            (func (export "codebot_alloc") (param i32) (result i32) i32.const 0)
            (func (export "codebot_run") (param i32 i32) (result i32) {body})
            (func (export "codebot_result_len") (result i32) i32.const {})
        )"#,
            raw.len()
        ))
        .unwrap()
    }
    fn package(
        root: &Path,
        id: &str,
        bytes: &[u8],
        permissions: Vec<PluginPermission>,
    ) -> PreparedPluginInstall {
        let folder = root.join(id);
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("plugin.wasm"), bytes).unwrap();
        let manifest = PluginManifest {
            manifest_version: 1,
            api_version: 1,
            id: id.into(),
            name: "Fixture extension".into(),
            version: "0.1.0".into(),
            description: "Fixture".into(),
            module: "plugin.wasm".into(),
            sha256: digest(bytes),
            permissions,
        };
        fs::write(
            folder.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        prepare_plugin_install(&folder.join("manifest.json")).unwrap()
    }
    #[test]
    fn the_rust_sample_installs_disabled_runs_with_selected_input_and_can_be_disabled_and_removed()
    {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("store");
        let source =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plugins/input-lines/manifest.json");
        let plan = prepare_plugin_install(&source).unwrap();
        assert_eq!(plan.manifest().id, "input-lines");
        install_plugin(&root, &plan).unwrap();
        assert!(!list_plugins(&root).unwrap()[0].enabled);
        assert!(run_plugin(
            &root,
            "input-lines",
            "one",
            false,
            dir.path(),
            &AtomicBool::new(false)
        )
        .is_err());
        set_plugin_enabled(&root, "input-lines", true).unwrap();
        let result = run_plugin(
            &root,
            "input-lines",
            "日本語\n二行\n三行",
            true,
            dir.path(),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(result.text.contains("3 行"));
        assert!(result.proposal.is_none());
        set_plugin_enabled(&root, "input-lines", false).unwrap();
        assert!(run_plugin(
            &root,
            "input-lines",
            "one",
            false,
            dir.path(),
            &AtomicBool::new(false)
        )
        .is_err());
        let removal = prepare_plugin_remove(&root, "input-lines").unwrap();
        remove_plugin(&removal).unwrap();
        assert!(list_plugins(&root).unwrap().is_empty());
        assert!(source.is_file());
    }
    #[test]
    fn guests_cannot_access_imports_network_selected_files_or_ungranted_actions() {
        assert!(module(
            &engine(),
            &wat::parse_str("(module (memory (export \"memory\") 1))").unwrap()
        )
        .err()
        .unwrap()
        .contains("API v1"));
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("store");
        let source = dir.path().join("source");
        let output = json!({"text":"proposal","proposal":{"name":"codebot_run_command","arguments":{"program":"python","args":["-c","open('must-not-run','w').write('bad')"],"timeout_seconds":5}}});
        let bytes = guest(output, "i32.const 65536");
        let plan = package(&source, "no-permissions", &bytes, vec![]);
        install_plugin(&root, &plan).unwrap();
        set_plugin_enabled(&root, "no-permissions", true).unwrap();
        assert!(run_plugin(
            &root,
            "no-permissions",
            "secret input",
            true,
            dir.path(),
            &AtomicBool::new(false)
        )
        .err()
        .unwrap()
        .contains("読む権限"));
        assert!(run_plugin(
            &root,
            "no-permissions",
            "secret input",
            false,
            dir.path(),
            &AtomicBool::new(false)
        )
        .err()
        .unwrap()
        .contains("許可していない"));
        let plan = package(
            &source,
            "command-proposal",
            &bytes,
            vec![PluginPermission::ProposeCommand],
        );
        install_plugin(&root, &plan).unwrap();
        set_plugin_enabled(&root, "command-proposal", true).unwrap();
        let result = run_plugin(
            &root,
            "command-proposal",
            "secret input",
            false,
            dir.path(),
            &AtomicBool::new(false),
        )
        .unwrap();
        assert!(result.proposal.is_some());
        assert!(!dir.path().join("must-not-run").exists());
        let bad =
            wat::parse_str(r#"(module (import "wasi_snapshot_preview1" "path_open" (func)))"#)
                .unwrap();
        let folder = source.join("imports");
        fs::create_dir_all(&folder).unwrap();
        fs::write(folder.join("plugin.wasm"), &bad).unwrap();
        let mut manifest = plan.manifest.clone();
        manifest.id = "imports".into();
        manifest.sha256 = digest(&bad);
        fs::write(
            folder.join("manifest.json"),
            serde_json::to_vec(&manifest).unwrap(),
        )
        .unwrap();
        assert!(prepare_plugin_install(&folder.join("manifest.json"))
            .err()
            .unwrap()
            .contains("import"));
        let plan = package(
            &source,
            "network-request",
            &bytes,
            vec![PluginPermission::Network],
        );
        assert!(!plan.manifest().supported());
        assert!(install_plugin(&root, &plan).is_err());
        assert!(!root.join("network-request").exists());
        let bytes = guest(
            json!({"text":"file proposal","proposal":{"name":"codebot_propose_changes","arguments":{"proposal":{"version":1,"changes":[{"operation":"create","path":"must-not-write.txt","content":"review first"}]}}}}),
            "i32.const 65536",
        );
        for (id, permissions, allowed) in [
            ("ungranted-write", vec![], false),
            (
                "reviewed-write",
                vec![PluginPermission::ProposeChanges],
                true,
            ),
        ] {
            let plan = package(&source, id, &bytes, permissions);
            install_plugin(&root, &plan).unwrap();
            set_plugin_enabled(&root, id, true).unwrap();
            let result = run_plugin(
                &root,
                id,
                "input",
                false,
                dir.path(),
                &AtomicBool::new(false),
            );
            assert_eq!(result.is_ok(), allowed);
            assert!(!dir.path().join("must-not-write.txt").exists());
        }
        for entry in fs::read_dir(root.join("command-proposal")).unwrap() {
            assert!(
                !String::from_utf8_lossy(&fs::read(entry.unwrap().path()).unwrap())
                    .contains("secret input")
            );
        }
    }
    #[test]
    fn fuel_memory_traps_and_bad_results_leave_the_host_and_next_extension_usable() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("store");
        let source = dir.path().join("source");
        let good = guest(
            json!({"text":"host still works","proposal":null}),
            "i32.const 65536",
        );
        let plan = package(&source, "good", &good, vec![]);
        install_plugin(&root, &plan).unwrap();
        set_plugin_enabled(&root, "good", true).unwrap();
        for (id, body) in [
            ("loop", "loop $again br $again end i32.const 65536"),
            ("start-loop", "START_LOOP"),
            ("trap", "unreachable"),
            ("memory", "i32.const 2000 memory.grow drop i32.const 65536"),
            ("pointer", "i32.const -1"),
        ] {
            let bytes = guest(
                json!({"text":"must not be delivered","proposal":null}),
                body,
            );
            let plan = package(&source, id, &bytes, vec![]);
            install_plugin(&root, &plan).unwrap();
            set_plugin_enabled(&root, id, true).unwrap();
            let error = run_plugin(
                &root,
                id,
                "input",
                false,
                dir.path(),
                &AtomicBool::new(false),
            )
            .err()
            .unwrap_or_else(|| panic!("expected guest failure: {id}"));
            if id == "loop" || id == "start-loop" {
                assert!(error.contains("実行上限"));
            }
            assert_eq!(
                run_plugin(
                    &root,
                    "good",
                    "input",
                    false,
                    dir.path(),
                    &AtomicBool::new(false)
                )
                .unwrap()
                .text,
                "host still works"
            );
        }
        assert!(run_plugin(
            &root,
            "good",
            "input",
            false,
            dir.path(),
            &AtomicBool::new(true)
        )
        .err()
        .unwrap()
        .contains("中止"));
        assert!(run_plugin(
            &root,
            "good",
            &"a".repeat(MAX_INPUT + 1),
            false,
            dir.path(),
            &AtomicBool::new(false)
        )
        .is_err());
    }
    #[test]
    fn reviewed_bytes_are_immutable_and_registry_tampering_or_changed_removal_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("store");
        let source = dir.path().join("source");
        let bytes = guest(
            json!({"text":"reviewed content","proposal":null}),
            "i32.const 65536",
        );
        let plan = package(&source, "reviewed", &bytes, vec![]);
        fs::write(source.join("reviewed/plugin.wasm"), b"changed after review").unwrap();
        install_plugin(&root, &plan).unwrap();
        set_plugin_enabled(&root, "reviewed", true).unwrap();
        assert_eq!(
            run_plugin(
                &root,
                "reviewed",
                "input",
                false,
                dir.path(),
                &AtomicBool::new(false)
            )
            .unwrap()
            .text,
            "reviewed content"
        );
        assert!(install_plugin(&root, &plan).is_err());
        let removal = prepare_plugin_remove(&root, "reviewed").unwrap();
        fs::write(root.join("reviewed/plugin.wasm"), b"tampered").unwrap();
        assert!(remove_plugin(&removal).is_err());
        assert!(list_plugins(&root).unwrap()[0].problem.is_some());
        assert!(run_plugin(
            &root,
            "reviewed",
            "input",
            false,
            dir.path(),
            &AtomicBool::new(false)
        )
        .is_err());
        let removal = prepare_plugin_remove(&root, "reviewed").unwrap();
        remove_plugin(&removal).unwrap();
        assert!(list_plugins(&root).unwrap().is_empty());
        install_plugin(&root, &plan).unwrap();
        let path = root.join("reviewed/manifest.json");
        let mut changed = plan.manifest.clone();
        changed.permissions.push(PluginPermission::ProposeCommand);
        fs::write(path, serde_json::to_vec(&changed).unwrap()).unwrap();
        assert!(set_plugin_enabled(&root, "reviewed", true).is_err());
        fs::write(root.join("reviewed/user-notes.txt"), b"keep").unwrap();
        assert!(prepare_plugin_remove(&root, "reviewed").is_err());
        let (_root, shared) = store(&root, false).unwrap();
        assert!(set_plugin_enabled(&root, "reviewed", false).is_err());
        drop(shared);
        for id in ["../outside", "con", "-option", "UPPERCASE"] {
            assert!(!valid_id(id));
        }
    }
}
