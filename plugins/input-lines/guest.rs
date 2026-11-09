//! CodeBot extension API v1 sample. No imports, files, commands, or networking.
//! Compile as wasm32-unknown-unknown, cdylib, panic=abort (see build_sample_plugin.py).
#![allow(static_mut_refs)]

static mut INPUT: Vec<u8> = Vec::new();
static mut OUTPUT: Vec<u8> = Vec::new();

#[no_mangle]
pub extern "C" fn codebot_alloc(length: i32) -> i32 {
    assert!((0..=256 * 1024).contains(&length));
    // The v1 host calls one instance sequentially and the guest has no imports.
    unsafe {
        INPUT.resize(length as usize, 0);
        INPUT.as_mut_ptr() as i32
    }
}

#[no_mangle]
pub extern "C" fn codebot_run(pointer: i32, length: i32) -> i32 {
    unsafe {
        assert_eq!(pointer, INPUT.as_ptr() as i32);
        assert_eq!(length as usize, INPUT.len());
        let lines = INPUT.iter().filter(|byte| **byte == b'\n').count()
            + usize::from(!INPUT.is_empty() && INPUT.last() != Some(&b'\n'));
        // Only a count is interpolated, so no user-supplied JSON escaping is needed.
        OUTPUT = format!(r#"{{"text":"入力全体は {lines} 行です。","proposal":null}}"#).into_bytes();
        OUTPUT.as_ptr() as i32
    }
}

#[no_mangle]
pub extern "C" fn codebot_result_len() -> i32 {
    unsafe { OUTPUT.len() as i32 }
}
