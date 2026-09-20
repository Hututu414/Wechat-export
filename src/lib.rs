#[cfg(not(all(target_os = "windows", target_arch = "x86_64")))]
compile_error!("wechat-export only supports Windows x64");

pub mod core;
