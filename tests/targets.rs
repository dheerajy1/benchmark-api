// Tests of saved targets: the targets file, request resolution, the custom-run switch,
// secret redaction, the compatibility of raw requests, the settings, and the
// configuration endpoints.

#[allow(dead_code)]
#[path = "benchmark/fake_repository.rs"]
mod fake_repository;

mod targets {
    #[allow(dead_code)]
    pub mod support;

    pub mod canary;
    pub mod compat;
    pub mod configuration;
    pub mod file;
    pub mod http;
    pub mod redaction;
    pub mod settings;
}
