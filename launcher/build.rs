use std::env;

fn main() {
    println!("cargo::rustc-check-cfg=cfg(has_mc_key)");

    // 当 MC_KEY 环境变量变化时，重跑 build.rs
    println!("cargo:rerun-if-env-changed=MC_KEY");
    println!("cargo:rerun-if-changed=build.rs");

    // 从环境变量读取密钥
    match env::var("MC_KEY") {
        Ok(real_secret) => {
            // 有真实密钥：注入编译期常量，并打上 cfg 标记
            println!("cargo:rustc-env=MC_KEY={}", real_secret);
            println!("cargo:rustc-cfg=has_mc_key");
        }
        Err(_) => {
            // 没设置密钥：用占位符保证编译成功，并输出警告
            println!("cargo:rustc-env=MC_KEY=local-dev-placeholder");
            println!("cargo:warning=未设置环境变量 MC_KEY，已使用占位符密钥");
        }
    }
}