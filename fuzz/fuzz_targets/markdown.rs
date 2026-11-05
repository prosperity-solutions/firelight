#![no_main]
libfuzzer_sys::fuzz_target!(|data: &[u8]| firelight_format::fuzz::markdown(data));
