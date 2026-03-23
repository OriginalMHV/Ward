use anyhow::Result;
use clap::Args;
use console::style;

use crate::config::auth;
use crate::config::manifest::Manifest;

#[derive(Args)]
pub struct DoctorCommand;

struct Check {
    name: &'static str,
    status: CheckStatus,
    detail: String,
}

enum CheckStatus {
    Pass,
    Warn,
    Fail,
}

impl DoctorCommand {
    pub async fn run(&self, config_path: Option<&str>) -> Result<()> {
        println!();
        println!("  {}", style("Ward Doctor").bold());
        println!("  {}", style("Diagnosing your setup...").dim());
        println!();

        let mut checks = vec![
            check_config(config_path),
            check_token(),
            check_gh_cli(),
            check_templates_dir(),
            check_audit_log(),
        ];

        let manifest = Manifest::load(config_path).ok();

        if let Some(ref m) = manifest {
            checks.push(check_org(m));
            checks.push(check_systems(m));
            checks.push(check_policies(m));
            checks.push(check_api_connectivity(config_path).await);
        }

        let mut pass = 0;
        let mut warn = 0;
        let mut fail = 0;

        for check in &checks {
            let icon = match check.status {
                CheckStatus::Pass => style("[ok]").green().bold(),
                CheckStatus::Warn => style("[!!]").yellow().bold(),
                CheckStatus::Fail => style("[x]").red().bold(),
            };
            println!(
                "  {} {:<30} {}",
                icon,
                check.name,
                style(&check.detail).dim()
            );

            match check.status {
                CheckStatus::Pass => pass += 1,
                CheckStatus::Warn => warn += 1,
                CheckStatus::Fail => fail += 1,
            }
        }

        println!();
        println!(
            "  {} passed, {} warnings, {} errors",
            style(pass).green().bold(),
            style(warn).yellow().bold(),
            style(fail).red().bold(),
        );

        if fail > 0 {
            println!();
            println!(
                "  {}",
                style("Fix the errors above to get Ward working.").red()
            );
        } else if warn > 0 {
            println!();
            println!(
                "  {}",
                style("Ward is functional but some things could be improved.").yellow()
            );
        } else {
            println!();
            println!("  {}", style("Everything looks good.").green());
        }

        println!();
        Ok(())
    }
}

fn check_config(path: Option<&str>) -> Check {
    let config_path = path.unwrap_or("ward.toml");
    if std::path::Path::new(config_path).exists() {
        match std::fs::read_to_string(config_path) {
            Ok(content) => match toml::from_str::<Manifest>(&content) {
                Ok(_) => Check {
                    name: "Configuration",
                    status: CheckStatus::Pass,
                    detail: format!("{config_path} found and valid"),
                },
                Err(e) => Check {
                    name: "Configuration",
                    status: CheckStatus::Fail,
                    detail: format!("parse error: {e}"),
                },
            },
            Err(e) => Check {
                name: "Configuration",
                status: CheckStatus::Fail,
                detail: format!("cannot read: {e}"),
            },
        }
    } else {
        Check {
            name: "Configuration",
            status: CheckStatus::Fail,
            detail: format!("{config_path} not found -- run 'ward init'"),
        }
    }
}

fn check_token() -> Check {
    match auth::resolve_token() {
        Ok(token) => {
            let prefix = &token[..std::cmp::min(8, token.len())];
            let source = if std::env::var("GH_TOKEN").is_ok() {
                "GH_TOKEN"
            } else if std::env::var("GITHUB_TOKEN").is_ok() {
                "GITHUB_TOKEN"
            } else {
                "gh auth token"
            };
            Check {
                name: "GitHub token",
                status: CheckStatus::Pass,
                detail: format!("{prefix}... via {source}"),
            }
        }
        Err(e) => Check {
            name: "GitHub token",
            status: CheckStatus::Fail,
            detail: format!("{e}"),
        },
    }
}

fn check_gh_cli() -> Check {
    match std::process::Command::new("gh").arg("--version").output() {
        Ok(output) if output.status.success() => {
            let version = String::from_utf8_lossy(&output.stdout);
            let version_line = version.lines().next().unwrap_or("unknown").trim();
            Check {
                name: "GitHub CLI",
                status: CheckStatus::Pass,
                detail: version_line.to_string(),
            }
        }
        _ => Check {
            name: "GitHub CLI",
            status: CheckStatus::Warn,
            detail: "not installed (optional, used for token fallback)".to_string(),
        },
    }
}

fn check_templates_dir() -> Check {
    let dir = dirs_path("templates");
    if dir.exists() {
        let count = std::fs::read_dir(&dir)
            .map(|entries| entries.filter_map(|e| e.ok()).count())
            .unwrap_or(0);
        Check {
            name: "Custom templates",
            status: CheckStatus::Pass,
            detail: format!("{} custom templates in {}", count, dir.display()),
        }
    } else {
        Check {
            name: "Custom templates",
            status: CheckStatus::Pass,
            detail: "no custom templates directory (using built-ins only)".to_string(),
        }
    }
}

fn check_audit_log() -> Check {
    let log = dirs_path("audit.log");
    if log.exists() {
        match std::fs::metadata(&log) {
            Ok(meta) => {
                let size_kb = meta.len() / 1024;
                let detail = if size_kb > 10_000 {
                    format!("{} KB -- consider rotating", size_kb)
                } else {
                    format!("{} KB", size_kb)
                };
                Check {
                    name: "Audit log",
                    status: if size_kb > 10_000 {
                        CheckStatus::Warn
                    } else {
                        CheckStatus::Pass
                    },
                    detail,
                }
            }
            Err(_) => Check {
                name: "Audit log",
                status: CheckStatus::Pass,
                detail: "exists but unreadable".to_string(),
            },
        }
    } else {
        Check {
            name: "Audit log",
            status: CheckStatus::Pass,
            detail: "not yet created (will be on first apply)".to_string(),
        }
    }
}

fn check_org(manifest: &Manifest) -> Check {
    if manifest.org.name.is_empty() {
        Check {
            name: "Organization",
            status: CheckStatus::Fail,
            detail: "org.name is empty in ward.toml".to_string(),
        }
    } else {
        Check {
            name: "Organization",
            status: CheckStatus::Pass,
            detail: manifest.org.name.clone(),
        }
    }
}

fn check_systems(manifest: &Manifest) -> Check {
    let count = manifest.systems.len();
    if count == 0 {
        Check {
            name: "Systems",
            status: CheckStatus::Warn,
            detail: "no systems defined -- add [[systems]] to ward.toml".to_string(),
        }
    } else {
        let names: Vec<&str> = manifest.systems.iter().map(|s| s.id.as_str()).collect();
        Check {
            name: "Systems",
            status: CheckStatus::Pass,
            detail: format!("{count} defined ({})", names.join(", ")),
        }
    }
}

fn check_policies(manifest: &Manifest) -> Check {
    let count = manifest.policies.len();
    if count == 0 {
        Check {
            name: "Policies",
            status: CheckStatus::Pass,
            detail: "none defined (optional)".to_string(),
        }
    } else {
        Check {
            name: "Policies",
            status: CheckStatus::Pass,
            detail: format!("{count} rules configured"),
        }
    }
}

async fn check_api_connectivity(config_path: Option<&str>) -> Check {
    let manifest = match Manifest::load(config_path) {
        Ok(m) => m,
        Err(_) => {
            return Check {
                name: "API connectivity",
                status: CheckStatus::Fail,
                detail: "cannot load config".to_string(),
            };
        }
    };

    let token = match auth::resolve_token() {
        Ok(t) => t,
        Err(_) => {
            return Check {
                name: "API connectivity",
                status: CheckStatus::Fail,
                detail: "no token available".to_string(),
            };
        }
    };

    let client = match reqwest::Client::builder()
        .default_headers({
            let mut headers = reqwest::header::HeaderMap::new();
            headers.insert(
                reqwest::header::AUTHORIZATION,
                reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))
                    .unwrap_or_else(|_| reqwest::header::HeaderValue::from_static("")),
            );
            headers.insert(
                reqwest::header::ACCEPT,
                reqwest::header::HeaderValue::from_static("application/vnd.github+json"),
            );
            headers.insert(
                reqwest::header::USER_AGENT,
                reqwest::header::HeaderValue::from_static("ward-cli/doctor"),
            );
            headers
        })
        .build()
    {
        Ok(c) => c,
        Err(_) => {
            return Check {
                name: "API connectivity",
                status: CheckStatus::Fail,
                detail: "cannot build HTTP client".to_string(),
            };
        }
    };

    let url = format!("https://api.github.com/orgs/{}", manifest.org.name);
    match client.get(&url).send().await {
        Ok(resp) => {
            let status = resp.status();
            let remaining = resp
                .headers()
                .get("x-ratelimit-remaining")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("?");

            if status.is_success() {
                Check {
                    name: "API connectivity",
                    status: CheckStatus::Pass,
                    detail: format!(
                        "authenticated to {} (rate limit: {} remaining)",
                        manifest.org.name, remaining
                    ),
                }
            } else if status.as_u16() == 401 {
                Check {
                    name: "API connectivity",
                    status: CheckStatus::Fail,
                    detail: "401 Unauthorized -- token is invalid or expired".to_string(),
                }
            } else if status.as_u16() == 403 {
                Check {
                    name: "API connectivity",
                    status: CheckStatus::Fail,
                    detail: format!(
                        "403 Forbidden -- token lacks access to {}",
                        manifest.org.name
                    ),
                }
            } else if status.as_u16() == 404 {
                Check {
                    name: "API connectivity",
                    status: CheckStatus::Fail,
                    detail: format!(
                        "org '{}' not found -- check org.name in ward.toml",
                        manifest.org.name
                    ),
                }
            } else {
                Check {
                    name: "API connectivity",
                    status: CheckStatus::Warn,
                    detail: format!("unexpected status: {status}"),
                }
            }
        }
        Err(e) => Check {
            name: "API connectivity",
            status: CheckStatus::Fail,
            detail: format!("connection failed: {e}"),
        },
    }
}

fn dirs_path(name: &str) -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home).join(".ward").join(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_check_config_missing_file() {
        let check = check_config(Some("/nonexistent/path/ward.toml"));
        assert!(matches!(check.status, CheckStatus::Fail));
        assert!(check.detail.contains("not found"));
    }

    #[test]
    fn test_check_config_valid_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ward.toml");
        std::fs::write(&path, "[org]\nname = \"test-org\"\n").unwrap();
        let check = check_config(Some(path.to_str().unwrap()));
        assert!(matches!(check.status, CheckStatus::Pass));
    }

    #[test]
    fn test_check_config_invalid_toml() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ward.toml");
        std::fs::write(&path, "this is not valid toml [[[").unwrap();
        let check = check_config(Some(path.to_str().unwrap()));
        assert!(matches!(check.status, CheckStatus::Fail));
        assert!(check.detail.contains("parse error"));
    }

    #[test]
    fn test_check_org_empty() {
        let manifest = Manifest::default();
        let check = check_org(&manifest);
        assert!(matches!(check.status, CheckStatus::Fail));
    }

    #[test]
    fn test_check_org_valid() {
        let mut manifest = Manifest::default();
        manifest.org.name = "my-org".to_string();
        let check = check_org(&manifest);
        assert!(matches!(check.status, CheckStatus::Pass));
        assert!(check.detail.contains("my-org"));
    }

    #[test]
    fn test_check_systems_none() {
        let manifest = Manifest::default();
        let check = check_systems(&manifest);
        assert!(matches!(check.status, CheckStatus::Warn));
    }

    #[test]
    fn test_check_systems_present() {
        let mut manifest = Manifest::default();
        manifest
            .systems
            .push(crate::config::manifest::SystemConfig {
                id: "backend".to_string(),
                name: "Backend".to_string(),
                exclude: vec![],
                repos: vec![],
                security: None,
                teams: vec![],
            });
        let check = check_systems(&manifest);
        assert!(matches!(check.status, CheckStatus::Pass));
        assert!(check.detail.contains("backend"));
    }

    #[test]
    fn test_check_policies_none() {
        let manifest = Manifest::default();
        let check = check_policies(&manifest);
        assert!(matches!(check.status, CheckStatus::Pass));
        assert!(check.detail.contains("none"));
    }
}
