// The persistence boundary for header values. The configuration is built by the production
// constructor from a request that carries real secret values, stored through the real
// repository, and then every column of the configuration table is inspected as text.

use std::{collections::BTreeMap, path::PathBuf};

use benchmark_api::{
    features::v1::benchmark::{
        configuration::RunConfiguration,
        model::{BenchmarkRequest, BuildCommand},
        request::{ReadinessRequest, StartupRequest, WorkloadRequest},
        run_origin::RunOrigin,
    },
    persistence::{
        postgres::repository::PgBenchmarkRepository,
        repository::{BenchmarkRepository, ConfigurationLookup},
    },
};

use super::{sample, support::setup};

const READINESS_SECRET: &str = "SECRET-READINESS-7f3a9c1e";
const WORKLOAD_SECRET: &str = "SECRET-WORKLOAD-91bc04de";
const PUBLIC_CLIENT_ID: &str = "public-client-id-5d2e";

// A request with real secret values in its headers, as a run would receive them
fn request_with_secrets() -> BenchmarkRequest {
    let mut readiness_headers = BTreeMap::new();
    readiness_headers.insert("X-Client-Id".to_string(), PUBLIC_CLIENT_ID.to_string());
    readiness_headers.insert("X-Client-Secret".to_string(), READINESS_SECRET.to_string());

    let mut workload_headers = BTreeMap::new();
    workload_headers.insert(
        "Authorization".to_string(),
        format!("Bearer {WORKLOAD_SECRET}"),
    );

    BenchmarkRequest {
        path: PathBuf::from("/absolute/path/to/project"),
        startup: Some(StartupRequest {
            command: BuildCommand {
                program: "bun".to_string(),
                args: vec!["run".to_string(), "dev".to_string()],
            },
            readiness: ReadinessRequest {
                readiness_type: "http".to_string(),
                host: "127.0.0.1".to_string(),
                port: 5000,
                path: "/health".to_string(),
                method: "GET".to_string(),
                expected_status: 200,
                headers: Some(readiness_headers),
            },
        }),
        workload: Some(WorkloadRequest {
            method: "GET".to_string(),
            path: "/items".to_string(),
            headers: workload_headers,
            requests: None,
            duration_seconds: Some(30),
            concurrency: 50,
        }),
        workspaces: Vec::new(),
    }
}

#[tokio::test]
async fn no_header_value_reaches_the_configuration_table() {
    let Some(database) = setup().await else {
        return;
    };

    let repository = PgBenchmarkRepository::new(database.pool.clone());
    let request = request_with_secrets();
    let origin = RunOrigin::Target {
        id: "demo".to_string(),
        name: "Demo API".to_string(),
    };

    // The production constructor, the same one the run paths use
    let configuration = RunConfiguration::from_request(&request, &origin);
    let result = sample::result();

    repository.save(&result, &configuration).await.unwrap();

    // Every column of every stored configuration row, rendered as text
    let rows: Vec<String> =
        sqlx::query_scalar("SELECT row_to_json(c)::text FROM benchmark_run_configurations c")
            .fetch_all(&database.pool)
            .await
            .unwrap();

    assert_eq!(rows.len(), 1, "exactly one configuration row is stored");

    let dumped = rows.join("\n");

    for value in [READINESS_SECRET, WORKLOAD_SECRET, PUBLIC_CLIENT_ID] {
        assert!(
            !dumped.contains(value),
            "a header value reached the configuration table: {value}"
        );
    }

    // The names are stored, so the configuration still describes the run
    assert!(dumped.contains("X-Client-Secret"), "{dumped}");
    assert!(dumped.contains("Authorization"), "{dumped}");

    // And the stored configuration reads back equal to what was saved, names only
    let lookup = repository.get_configuration(result.run_id).await.unwrap();

    assert_eq!(
        lookup,
        ConfigurationLookup::Recorded(Box::new(configuration))
    );

    database.teardown().await;
}
