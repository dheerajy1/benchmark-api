// Tests of the live-run core: the execution slot, the bounded event log, the registry,
// the supervisor and the shared guard of the legacy endpoint.

#[allow(dead_code)]
#[path = "benchmark/fake_repository.rs"]
mod fake_repository;

#[allow(dead_code)]
#[path = "benchmark/comparison_sample.rs"]
mod comparison_sample;

mod live {
    pub mod guard_http;
    pub mod log;
    pub mod overhead;
    pub mod registry;
    pub mod runs;
    pub mod samples;
    pub mod slot;
    pub mod support;
}
