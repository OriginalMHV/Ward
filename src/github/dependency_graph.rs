use serde::{Deserialize, Serialize};

use super::Client;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DependencyGraphStatus {
    Available,
    Empty,
    Unavailable,
    Unknown,
}

impl Default for DependencyGraphStatus {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyGraphAudit {
    pub status: DependencyGraphStatus,
    pub reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sbom_generated_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_count: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dependency_count: Option<usize>,
}

impl DependencyGraphAudit {
    fn available(sbom: SbomDocument) -> Self {
        let package_count = sbom.packages.len();
        let dependency_count = sbom
            .packages
            .iter()
            .filter(|pkg| pkg.spdx_id.as_deref() != Some("SPDXRef-Repository"))
            .count();
        let created_at = sbom.creation_info.and_then(|info| info.created);

        if dependency_count > 0 {
            Self {
                status: DependencyGraphStatus::Available,
                reason: format!(
                    "SBOM export succeeded with {dependency_count} dependency package(s)"
                ),
                sbom_generated_at: created_at,
                package_count: Some(package_count),
                dependency_count: Some(dependency_count),
            }
        } else {
            Self {
                status: DependencyGraphStatus::Empty,
                reason: "SBOM export succeeded but contains no dependency packages".to_owned(),
                sbom_generated_at: created_at,
                package_count: Some(package_count),
                dependency_count: Some(0),
            }
        }
    }

    fn unavailable(reason: impl Into<String>) -> Self {
        Self {
            status: DependencyGraphStatus::Unavailable,
            reason: reason.into(),
            sbom_generated_at: None,
            package_count: None,
            dependency_count: None,
        }
    }

    fn unknown(reason: impl Into<String>) -> Self {
        Self {
            status: DependencyGraphStatus::Unknown,
            reason: reason.into(),
            sbom_generated_at: None,
            package_count: None,
            dependency_count: None,
        }
    }
}

#[derive(Debug, Deserialize)]
struct SbomResponse {
    sbom: SbomDocument,
}

#[derive(Debug, Deserialize)]
struct SbomDocument {
    #[serde(rename = "creationInfo")]
    creation_info: Option<SbomCreationInfo>,
    #[serde(default)]
    packages: Vec<SbomPackage>,
}

#[derive(Debug, Deserialize)]
struct SbomCreationInfo {
    #[serde(default)]
    created: Option<String>,
}

#[derive(Debug, Deserialize)]
struct SbomPackage {
    #[serde(rename = "SPDXID")]
    #[serde(default)]
    spdx_id: Option<String>,
}

impl Client {
    /// Audit whether GitHub can currently export dependency graph data as an SBOM.
    pub async fn audit_dependency_graph(&self, repo: &str) -> DependencyGraphAudit {
        match self
            .get(&format!("/repos/{}/{repo}/dependency-graph/sbom", self.org))
            .await
        {
            Ok(resp) => match resp.status().as_u16() {
                200 => match resp.json::<SbomResponse>().await {
                    Ok(body) => DependencyGraphAudit::available(body.sbom),
                    Err(err) => DependencyGraphAudit::unknown(format!(
                        "GitHub returned an SBOM response Ward could not parse: {err}"
                    )),
                },
                404 => DependencyGraphAudit::unavailable(
                    "GitHub could not export an SBOM for this repository",
                ),
                403 => DependencyGraphAudit::unknown(
                    "GitHub denied SBOM export; token may be missing Contents read access",
                ),
                status => {
                    let body = resp.text().await.unwrap_or_default();
                    tracing::warn!("SBOM export for {repo} returned HTTP {status}: {body}");
                    DependencyGraphAudit::unknown(format!(
                        "GitHub returned HTTP {status} when Ward tried to export an SBOM"
                    ))
                }
            },
            Err(err) => DependencyGraphAudit::unknown(format!(
                "Ward could not call GitHub's SBOM export endpoint: {err}"
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn available_status_when_dependencies_exist() {
        let audit = DependencyGraphAudit::available(SbomDocument {
            creation_info: Some(SbomCreationInfo {
                created: Some("2026-04-19T10:00:00Z".to_owned()),
            }),
            packages: vec![
                SbomPackage {
                    spdx_id: Some("SPDXRef-Repository".to_owned()),
                },
                SbomPackage {
                    spdx_id: Some("SPDXRef-Package-1".to_owned()),
                },
            ],
        });

        assert_eq!(audit.status, DependencyGraphStatus::Available);
        assert_eq!(audit.package_count, Some(2));
        assert_eq!(audit.dependency_count, Some(1));
    }

    #[test]
    fn empty_status_when_only_repository_package_exists() {
        let audit = DependencyGraphAudit::available(SbomDocument {
            creation_info: None,
            packages: vec![SbomPackage {
                spdx_id: Some("SPDXRef-Repository".to_owned()),
            }],
        });

        assert_eq!(audit.status, DependencyGraphStatus::Empty);
        assert_eq!(audit.package_count, Some(1));
        assert_eq!(audit.dependency_count, Some(0));
    }
}
