#![no_main]

use libfuzzer_sys::fuzz_target;

#[cfg(target_os = "linux")]
use thinws_adapter_linux::fuzz_workspace_ownership_document;
#[cfg(target_os = "macos")]
use thinws_adapter_macos::fuzz_workspace_ownership_document;

fuzz_target!(|bytes: &[u8]| {
    fuzz_workspace_ownership_document(bytes);
});
