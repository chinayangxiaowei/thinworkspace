#![no_main]

use libfuzzer_sys::fuzz_target;

#[cfg(target_os = "linux")]
fuzz_target!(|bytes: &[u8]| {
    thinws_adapter_linux::fuzz_mountinfo(bytes);
});

// Cargo's macOS fuzz build checks every declared target; the release runner
// excludes this Linux-only target from execution on macOS.
#[cfg(not(target_os = "linux"))]
fuzz_target!(|_bytes: &[u8]| {});
