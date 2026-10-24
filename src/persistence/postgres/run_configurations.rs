use chrono::{DateTime, Utc};
use sqlx::{PgConnection, PgPool, Row, postgres::PgRow, types::Json};
use uuid::Uuid;

use crate::{
    features::v1::benchmark::configuration::{
        ReadinessConfiguration, ReadinessSummary, RunConfiguration, StartupConfiguration,
        StartupSummary, WorkloadConfiguration,
    },
    persistence::{
        error::PersistenceError,
        repository::{ConfigurationLookup, StoredConfigurationRun},
    },
};

// The configuration columns, read in one place so every query returns the same shape
const SELECT_COLUMNS: &str = "c.source, c.target_id, c.target_name, c.target_path, \
    c.startup_program, c.startup_args, c.readiness_type, c.readiness_host, c.readiness_port, \
    c.readiness_path, c.readiness_method, c.readiness_expected_status, \
    c.readiness_header_names, c.workload_method, c.workload_path, c.workload_mode, \
    c.workload_requests, c.workload_duration_seconds, c.workload_concurrency, \
    c.workload_header_names";

// Writes the configuration of one run on the caller's transaction, so it is stored
// with the run or not at all
pub async fn insert(
    connection: &mut PgConnection,
    run_id: Uuid,
    configuration: &RunConfiguration,
) -> Result<(), PersistenceError> {
    let startup = configuration.startup.as_ref();
    let readiness = startup.map(|startup| &startup.readiness);
    let workload = configuration.workload.as_ref();

    let requests = to_bigint(workload.and_then(|workload| workload.requests))?;
    let duration = to_bigint(workload.and_then(|workload| workload.duration_seconds))?;
    let concurrency = to_bigint(workload.map(|workload| workload.concurrency))?;

    sqlx::query(
        "INSERT INTO benchmark_run_configurations (
            run_id, source, target_id, target_name, target_path,
            startup_program, startup_args,
            readiness_type, readiness_host, readiness_port, readiness_path,
            readiness_method, readiness_expected_status, readiness_header_names,
            workload_method, workload_path, workload_mode, workload_requests,
            workload_duration_seconds, workload_concurrency, workload_header_names
         ) VALUES (
            $1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14,
            $15, $16, $17, $18, $19, $20, $21
         )",
    )
    .bind(run_id)
    .bind(configuration.source)
    .bind(configuration.target_id.as_deref())
    .bind(configuration.target_name.as_deref())
    .bind(configuration.target_path.as_str())
    .bind(startup.map(|startup| startup.program.as_str()))
    .bind(startup.map(|startup| startup.args.clone()))
    .bind(readiness.map(|readiness| readiness.readiness_type.as_str()))
    .bind(readiness.map(|readiness| readiness.host.as_str()))
    .bind(readiness.map(|readiness| i32::from(readiness.port)))
    .bind(readiness.map(|readiness| readiness.path.as_str()))
    .bind(readiness.map(|readiness| readiness.method.as_str()))
    .bind(readiness.map(|readiness| i32::from(readiness.expected_status)))
    .bind(readiness.and_then(|readiness| readiness.header_names.clone()))
    .bind(workload.map(|workload| workload.method.as_str()))
    .bind(workload.map(|workload| workload.path.as_str()))
    .bind(workload.map(|workload| workload.mode))
    .bind(requests)
    .bind(duration)
    .bind(concurrency)
    .bind(workload.map(|workload| workload.header_names.clone()))
    .execute(&mut *connection)
    .await?;

    Ok(())
}

// The configuration of one run. A missing run and a run without a recorded configuration
// are different answers.
pub async fn get(pool: &PgPool, run_id: Uuid) -> Result<ConfigurationLookup, PersistenceError> {
    let sql = format!(
        "SELECT {SELECT_COLUMNS} FROM benchmark_runs r \
         LEFT JOIN benchmark_run_configurations c ON c.run_id = r.id \
         WHERE r.id = $1"
    );

    let row = sqlx::query(&sql).bind(run_id).fetch_optional(pool).await?;

    Ok(match row {
        None => ConfigurationLookup::RunMissing,
        Some(row) => match row.try_get::<Option<String>, _>("source")? {
            None => ConfigurationLookup::NotRecorded,
            Some(_) => ConfigurationLookup::Recorded(from_row(&row)?),
        },
    })
}

// The newest runs, newest first. Each run carries its path and start command: from the
// configuration when recorded, and from the existing run and startup tables otherwise.
pub async fn list(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<StoredConfigurationRun>, PersistenceError> {
    let sql = format!(
        "SELECT r.id AS run_id, r.created_at, r.target_path AS run_target_path, \
         s.program AS legacy_program, s.args AS legacy_args, \
         s.readiness_type AS legacy_readiness_type, s.readiness_host AS legacy_readiness_host, \
         s.readiness_port AS legacy_readiness_port, s.readiness_path AS legacy_readiness_path, \
         s.readiness_method AS legacy_readiness_method, \
         s.readiness_expected_status AS legacy_readiness_expected_status, \
         {SELECT_COLUMNS} \
         FROM benchmark_runs r \
         LEFT JOIN benchmark_run_configurations c ON c.run_id = r.id \
         LEFT JOIN benchmark_startups s ON s.benchmark_run_id = r.id \
         ORDER BY r.created_at DESC, r.id DESC LIMIT $1"
    );

    let rows = sqlx::query(&sql).bind(limit).fetch_all(pool).await?;

    let mut runs = Vec::with_capacity(rows.len());

    for row in &rows {
        let run_id: Uuid = row.try_get("run_id")?;
        let created_at: DateTime<Utc> = row.try_get("created_at")?;

        let (path, startup, configuration) = match row.try_get::<Option<String>, _>("source")? {
            None => (
                row.try_get::<String, _>("run_target_path")?,
                legacy_startup(row)?,
                None,
            ),
            Some(_) => {
                let configuration = from_row(row)?;
                let path = configuration.target_path.clone();
                let startup = configuration.startup.clone().map(StartupSummary::from);

                (path, startup, Some(configuration))
            }
        };

        runs.push(StoredConfigurationRun {
            run_id,
            created_at,
            path,
            startup,
            configuration,
        });
    }

    Ok(runs)
}

// The start command and readiness of a run stored before configurations were recorded,
// from its startup row. Header names are not known for these runs, so they stay null.
fn legacy_startup(row: &PgRow) -> Result<Option<StartupSummary>, PersistenceError> {
    let program: Option<String> = row.try_get("legacy_program")?;
    let args: Option<Json<Vec<String>>> = row.try_get("legacy_args")?;
    let readiness_type: Option<String> = row.try_get("legacy_readiness_type")?;

    let readiness = match readiness_type {
        None => None,
        Some(readiness_type) => Some(ReadinessSummary {
            readiness_type,
            host: required_string(row, "legacy_readiness_host")?,
            port: required_u16(row, "legacy_readiness_port")?,
            path: required_string(row, "legacy_readiness_path")?,
            method: required_string(row, "legacy_readiness_method")?,
            expected_status: required_u16(row, "legacy_readiness_expected_status")?,
            header_names: None,
        }),
    };

    if program.is_none() && args.is_none() && readiness.is_none() {
        return Ok(None);
    }

    Ok(Some(StartupSummary {
        program,
        args: args.map(|Json(args)| args),
        readiness,
    }))
}

fn from_row(row: &PgRow) -> Result<RunConfiguration, PersistenceError> {
    let source = match required_string(row, "source")?.as_str() {
        "custom" => "custom",
        "target" => "target",
        other => {
            return Err(PersistenceError::InvalidResult(format!(
                "unknown configuration source {other}"
            )));
        }
    };

    let startup = match row.try_get::<Option<String>, _>("startup_program")? {
        None => None,
        Some(program) => Some(StartupConfiguration {
            program,
            args: row
                .try_get::<Option<Vec<String>>, _>("startup_args")?
                .unwrap_or_default(),
            readiness: ReadinessConfiguration {
                readiness_type: required_string(row, "readiness_type")?,
                host: required_string(row, "readiness_host")?,
                port: required_u16(row, "readiness_port")?,
                path: required_string(row, "readiness_path")?,
                method: required_string(row, "readiness_method")?,
                expected_status: required_u16(row, "readiness_expected_status")?,
                header_names: row.try_get("readiness_header_names")?,
            },
        }),
    };

    let workload = match row.try_get::<Option<String>, _>("workload_mode")? {
        None => None,
        Some(mode) => {
            let mode = match mode.as_str() {
                "requests" => "requests",
                "duration" => "duration",
                other => {
                    return Err(PersistenceError::InvalidResult(format!(
                        "unknown workload mode {other}"
                    )));
                }
            };

            Some(WorkloadConfiguration {
                method: required_string(row, "workload_method")?,
                path: required_string(row, "workload_path")?,
                mode,
                requests: optional_u64(row, "workload_requests")?,
                duration_seconds: optional_u64(row, "workload_duration_seconds")?,
                concurrency: required_u64(row, "workload_concurrency")?,
                header_names: row
                    .try_get::<Option<Vec<String>>, _>("workload_header_names")?
                    .unwrap_or_default(),
            })
        }
    };

    Ok(RunConfiguration {
        source,
        target_id: row.try_get("target_id")?,
        target_name: row.try_get("target_name")?,
        target_path: required_string(row, "target_path")?,
        startup,
        workload,
    })
}

fn to_bigint(value: Option<u64>) -> Result<Option<i64>, PersistenceError> {
    value
        .map(|value| {
            i64::try_from(value).map_err(|_| {
                PersistenceError::InvalidResult(format!("{value} is too large to store"))
            })
        })
        .transpose()
}

fn missing(column: &str) -> PersistenceError {
    PersistenceError::InvalidResult(format!("{column} is missing from a stored configuration"))
}

fn required_string(row: &PgRow, column: &str) -> Result<String, PersistenceError> {
    row.try_get::<Option<String>, _>(column)?
        .ok_or_else(|| missing(column))
}

fn required_u16(row: &PgRow, column: &str) -> Result<u16, PersistenceError> {
    let value = row
        .try_get::<Option<i32>, _>(column)?
        .ok_or_else(|| missing(column))?;

    u16::try_from(value)
        .map_err(|_| PersistenceError::InvalidResult(format!("{column} is out of range")))
}

fn optional_u64(row: &PgRow, column: &str) -> Result<Option<u64>, PersistenceError> {
    row.try_get::<Option<i64>, _>(column)?
        .map(|value| {
            u64::try_from(value)
                .map_err(|_| PersistenceError::InvalidResult(format!("{column} is out of range")))
        })
        .transpose()
}

fn required_u64(row: &PgRow, column: &str) -> Result<u64, PersistenceError> {
    optional_u64(row, column)?.ok_or_else(|| missing(column))
}
