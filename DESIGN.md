# CodeBot desktop interface

The desktop app is a local development assistant with CLI parity. Processing can use local rule-based diagnostics, a user-specified GGUF or safetensors model served by an owned llama.cpp or Transformers process, or configured AI APIs. The conversation is a visual history; each API request sends only its current input. No example prompts or invented integrations belong in the UI.

## Direction

A focused conversation workspace replaces the earlier segmented form. The desktop app belongs beside an editor: white surfaces with blue actions and selections, restrained sans-serif typography, readable message measure, one compact composer. Avoid promotional headings, repeated cards, bright decorative accents, and full-width form rows. Empty and populated conversations have different compositions.

## Shared tokens

| Role | Value |
|---|---|
| Shell/sidebar | #F4F7FB |
| Content | #FFFFFF |
| Raised input | #F8FAFD |
| Border | #D2DCE9 |
| Primary text | #1C283A |
| Secondary text | #58687D |
| Primary action | #005FB8, white icon |
| Primary action hover | #004E99 |
| Selected row/user message | #E5F0FD |
| Disabled action | #DBE3EE |
| Maximum message width | 680 logical pixels |
| Maximum composer width | 740 logical pixels |
| Composer padding | 18px horizontal, 16px vertical |
| Sidebar width | 224 logical pixels |
| Main text | LINE Seed JP Regular 400 (default), 15–16 logical pixels |
| Conversation line height | 22px input, 25px response |
| Labels | 12–14 logical pixels |
| Empty title | 28 logical pixels |

These tokens live in `crates/codebot-gui/src/main.rs`. The UI has no decorative motion. Keep native egui keyboard and focus behavior for controls; authored line icons are drawn inside actual buttons with accessible names.

## Layout and behavior

- Place a drawn plus icon before the new-chat button label. The sidebar is a left-aligned session list, with settings at the bottom. Keep titles on one line and ellipsize to the available width. Selection uses surface contrast, not an extra badge or colored rule.
- A quiet header contains session context and new-chat/settings actions. Omit a global status bar and result counts.
- An empty conversation places a short heading above the composer in the central reading area. A populated conversation anchors the composer near the bottom and scrolls history independently.
- User messages use a narrow right-aligned surface; responses use plain text on the conversation background. Copy feedback appears beside the copy action.
- Use a 20px corner radius for the composer frame. The composer text area is exactly two UI font rows high. Place send on its right, not on a separate row. Long logs scroll internally; the frame must not grow with input. Keep the disclosure and keyboard hint outside the frame below it.
- Store drafts per session. Switching chats must not lose unfinished input. Async results always return to their original session.
- Settings use the same reading column, not a floating utility window. Changes apply at the next send; Escape returns to the chat.
- Below 900px, replace the sidebar with a session menu. Support the minimum 700×540 viewport without shrinking text.
- Below the composer, name the active API destination or state that local processing sends nothing externally. Never show a non-networked claim for API mode. Do not repeat it across header and sidebar.

## Validation scope

Linux virtual-display captures cover 1180×800 and 700×540, empty and populated history, settings, and multiline input. Tests cover core/CLI behavior and GUI session state. Windows/macOS runtime and assistive technology remain unverified.

Use LINE Seed JP Regular 400 for all UI text by default, including emphasis labels. Settings lets the user choose BIZ UDPGothic (Regular body, actual Bold section labels). Do not simulate weights. Apply the choice immediately and persist it in `gui-preferences.toml` beside the OS-standard application config file; this file is independent of the diagnostic TOML path. Report read/write errors in Settings. Keep Noto assets as historical material, not active UI fonts. Each active font has a bundled OFL license under `assets/fonts/`.

The default theme is explicitly Light, independent of the OS preference. Use a blue background with a white arrow for enabled send; disabled send uses a muted gray-blue background. Keep dark text on the light chat surface, and blue focus indicators.

API settings live at the top of Settings: service, model ID, base URL, protocol for custom endpoints, masked in-memory API key, environment variable name, output limit and timeout. Persist only non-secret API profiles. Clear the in-memory key when changing provider or endpoint. Show save errors beside the save action.

Local AI has a separate configuration panel for a model path or HF file URL, verification/download progress, llama-server path, load/stop controls, and advanced runtime settings. Model-name presets without verified sources must be visibly marked unverified and must never invent a repository URL. Downloads are explicit; inference sends data only to the owned loopback server. Keep the existing no-example-prompts rule.

Local AI lets the user select llama.cpp or Transformers. A Transformers source is a model folder or HF repository ID/URL, never a single GGUF URL. Show the Python interpreter and device selection for this mode. Preserve the existing llama.cpp settings for backward compatibility.
