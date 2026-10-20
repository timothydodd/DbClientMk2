//! Argument lists for the SqlPackage CLI (DACPAC extract / publish / script). Locating and
//! running the tool lives in the engine crate; this is the pure part.

pub const INSTALL_HINT: &str = "Install with: dotnet tool install -g microsoft.sqlpackage";

pub fn extract_args(connection_string: &str, target_file: &str, include_data: bool) -> Vec<String> {
    vec![
        "/Action:Extract".into(),
        format!("/SourceConnectionString:{connection_string}"),
        format!("/TargetFile:{target_file}"),
        format!("/p:ExtractAllTableData={}", if include_data { "True" } else { "False" }),
        "/p:VerifyExtraction=False".into(),
    ]
}

/// The target database is the connection string's Initial Catalog (SqlPackage creates it when
/// missing); `/TargetDatabaseName` can't be combined with `/TargetConnectionString`.
pub fn publish_args(source_file: &str, connection_string: &str, block_on_data_loss: bool) -> Vec<String> {
    vec![
        "/Action:Publish".into(),
        format!("/SourceFile:{source_file}"),
        format!("/TargetConnectionString:{connection_string}"),
        format!("/p:BlockOnPossibleDataLoss={}", if block_on_data_loss { "True" } else { "False" }),
    ]
}

pub fn script_args(source_file: &str, connection_string: &str, output_file: &str, block_on_data_loss: bool) -> Vec<String> {
    vec![
        "/Action:Script".into(),
        format!("/SourceFile:{source_file}"),
        format!("/TargetConnectionString:{connection_string}"),
        format!("/OutputPath:{output_file}"),
        format!("/p:BlockOnPossibleDataLoss={}", if block_on_data_loss { "True" } else { "False" }),
    ]
}

/// The command line with connection strings (which may carry a password) hidden, for the log.
pub fn redact(args: &[String]) -> String {
    args.iter()
        .map(|a| {
            if a.to_lowercase().contains("connectionstring:") {
                let idx = a.find(':').unwrap_or(a.len() - 1);
                format!("{}<redacted>", &a[..=idx])
            } else {
                a.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_connection_strings() {
        let args = publish_args("a.dacpac", "Server=x;Password=hunter2", true);
        let r = redact(&args);
        assert!(!r.contains("hunter2"));
        assert!(r.contains("/TargetConnectionString:<redacted>"));
        assert!(r.contains("/SourceFile:a.dacpac"));
    }
}
