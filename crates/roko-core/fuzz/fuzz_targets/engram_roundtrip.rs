#![no_main]

use libfuzzer_sys::fuzz_target;
use roko_core::Engram;

fuzz_target!(|data: &[u8]| {
    if let Ok(text) = std::str::from_utf8(data)
        && let Ok(engram) = serde_json::from_str::<Engram>(text)
    {
        let _ = serde_json::to_vec(&engram);
        let _ = engram.content_hash();
    }
});
