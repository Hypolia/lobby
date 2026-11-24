use std::path::PathBuf;

fn migration_files() -> Vec<(String, String)> {
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("migrations");
    let mut files: Vec<(String, String)> = std::fs::read_dir(&dir)
        .expect("migrations dir")
        .map(|entry| entry.expect("entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "sql"))
        .map(|path| {
            let name = path
                .file_name()
                .expect("name")
                .to_string_lossy()
                .into_owned();
            (name, std::fs::read_to_string(&path).expect("read"))
        })
        .collect();
    files.sort();
    files
}

fn statements(sql: &str) -> Vec<String> {
    sql.lines()
        .filter(|line| !line.trim_start().starts_with("--"))
        .collect::<Vec<_>>()
        .join(" ")
        .split(';')
        .map(|stmt| {
            stmt.split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_uppercase()
        })
        .filter(|stmt| !stmt.is_empty())
        .collect()
}

fn violation(stmt: &str) -> Option<&'static str> {
    const FORBIDDEN: [(&str, &str); 9] = [
        ("DROP ", "DROP"),
        ("TRUNCATE", "TRUNCATE"),
        ("DELETE ", "DELETE"),
        ("RENAME", "RENAME"),
        (" TYPE ", "ALTER COLUMN TYPE"),
        ("SET NOT NULL", "SET NOT NULL"),
        ("ALTER TABLE", "ALTER TABLE (revue manuelle requise)"),
        ("UPDATE ", "UPDATE"),
        ("SET DATA", "SET DATA TYPE"),
    ];
    if stmt.starts_with("ALTER TABLE")
        && stmt.contains("ADD COLUMN")
        && !stmt.contains("NOT NULL")
        && !stmt.contains("DROP ")
        && !stmt.contains("RENAME")
    {
        return None;
    }
    if stmt.starts_with("ALTER TABLE")
        && stmt.contains("ADD COLUMN")
        && stmt.contains("NOT NULL")
        && stmt.contains("DEFAULT")
        && !stmt.contains("DROP ")
        && !stmt.contains("RENAME")
    {
        return None;
    }
    FORBIDDEN
        .iter()
        .find(|(needle, _)| stmt.contains(needle))
        .map(|(_, label)| *label)
}

#[test]
fn plc_7_migrations_are_expand_only() {
    let files = migration_files();
    assert!(!files.is_empty());
    for (name, sql) in &files {
        for stmt in statements(sql) {
            if let Some(label) = violation(&stmt) {
                panic!("{name} : instruction non compatible N-1 ({label}) : {stmt}");
            }
        }
    }
}

#[test]
fn plc_7_the_checker_rejects_contracting_statements() {
    for bad in [
        "DROP TABLE player_sessions",
        "ALTER TABLE player_sessions DROP COLUMN session",
        "ALTER TABLE player_sessions RENAME COLUMN session TO data",
        "ALTER TABLE player_sessions ALTER COLUMN session TYPE text",
        "ALTER TABLE player_sessions ALTER COLUMN session SET NOT NULL",
        "ALTER TABLE player_sessions ADD COLUMN x text NOT NULL",
        "DELETE FROM player_sessions",
    ] {
        let stmt = statements(bad).remove(0);
        assert!(violation(&stmt).is_some(), "{bad} devrait être refusé");
    }
    for good in [
        "CREATE TABLE IF NOT EXISTS t (id int)",
        "ALTER TABLE player_sessions ADD COLUMN note text",
        "ALTER TABLE player_sessions ADD COLUMN note text NOT NULL DEFAULT ''",
        "CREATE INDEX IF NOT EXISTS idx ON player_sessions (realm)",
    ] {
        let stmt = statements(good).remove(0);
        assert!(violation(&stmt).is_none(), "{good} devrait passer");
    }
}
