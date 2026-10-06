use clap::{Args, ValueEnum};

use crate::reconcile::unified::Category;

/// Which repositories a command targets. `--system` and `--repo` narrow the
/// manifest scope. `--org` overrides the organization in `ward.toml`.
#[derive(Args, Debug, Default, Clone, PartialEq, Eq)]
pub struct TargetArgs {
    /// GitHub organization (overrides ward.toml)
    #[arg(long)]
    pub org: Option<String>,

    /// Narrow the run to one system (for example backend)
    #[arg(long)]
    pub system: Option<String>,

    /// Narrow the run to one repository
    #[arg(long)]
    pub repo: Option<String>,
}

/// The target of `ward repos list`: it lists by organization or system only.
#[derive(Args, Debug, Default, Clone, PartialEq, Eq)]
pub struct ListTargetArgs {
    /// GitHub organization (overrides ward.toml)
    #[arg(long)]
    pub org: Option<String>,

    /// List only the repositories of one system
    #[arg(long)]
    pub system: Option<String>,
}

/// The output format of a command.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, ValueEnum)]
pub enum Format {
    /// Human-readable text
    #[default]
    #[value(alias = "table")]
    Text,
    /// Machine-readable JSON on stdout
    Json,
}

// `--format text|json`, plus the deprecated `--json` flag.
#[derive(Args, Debug, Default, Clone, PartialEq, Eq)]
pub struct OutputArgs {
    /// Output format
    #[arg(long, value_enum, default_value_t = Format::Text)]
    pub format: Format,

    /// Deprecated. Use `--format json`
    #[arg(long, hide = true)]
    json: bool,
}

impl OutputArgs {
    /// The format to use. The deprecated `--json` flag selects JSON and prints a notice to stderr.
    pub fn resolve(&self) -> Format {
        if self.json {
            eprintln!(
                "warning: '--json' is deprecated and will be removed in 0.6.0; use '--format json'"
            );
            return Format::Json;
        }
        self.format
    }

    pub fn is_json(&self) -> bool {
        self.resolve() == Format::Json
    }
}

/// The `--category` filter shared by every manifest command.
#[derive(Args, Debug, Default, Clone)]
pub struct CategoryArgs {
    /// Limit to these categories. Repeat the flag or separate values with commas.
    #[arg(
        long = "category",
        value_name = "CATEGORY",
        value_enum,
        value_delimiter = ',',
        ignore_case = true
    )]
    pub categories: Vec<Category>,
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::*;

    #[derive(Parser)]
    struct Probe {
        #[command(flatten)]
        category: CategoryArgs,
    }

    fn parse(args: &[&str]) -> Result<Vec<Category>, clap::Error> {
        Probe::try_parse_from(std::iter::once("probe").chain(args.iter().copied()))
            .map(|probe| probe.category.categories)
    }

    #[derive(Parser)]
    struct OutputProbe {
        #[command(flatten)]
        output: OutputArgs,
    }

    #[test]
    fn format_defaults_to_text_and_accepts_table_as_a_hidden_alias() {
        let default = OutputProbe::try_parse_from(["probe"]).unwrap();
        assert_eq!(default.output.format, Format::Text);

        let table = OutputProbe::try_parse_from(["probe", "--format", "table"]).unwrap();
        assert_eq!(table.output.format, Format::Text);

        let json = OutputProbe::try_parse_from(["probe", "--format", "json"]).unwrap();
        assert!(json.output.is_json());

        assert!(OutputProbe::try_parse_from(["probe", "--format", "yaml"]).is_err());
    }

    #[test]
    fn deprecated_json_flag_selects_json() {
        let probe = OutputProbe::try_parse_from(["probe", "--json"]).unwrap();
        assert_eq!(probe.output.resolve(), Format::Json);
    }

    #[test]
    fn table_is_not_listed_as_a_value() {
        use clap::CommandFactory;
        let help = OutputProbe::command().render_long_help().to_string();
        assert!(help.contains("text"), "{help}");
        assert!(!help.contains("table"), "{help}");
        assert!(!help.contains("--json"), "{help}");
    }

    #[test]
    fn canonical_names_parse() {
        let all = parse(&[
            "--category",
            "repository,files,security,actions,environments,access,integrations,rulesets,branch-protection",
        ])
        .unwrap();
        assert_eq!(all.len(), 9);
    }

    #[test]
    fn aliases_parse_to_their_category() {
        for (alias, expected) in [
            ("repo", Category::Repository),
            ("general", Category::Repository),
            ("file", Category::Files),
            ("ruleset", Category::Rulesets),
            ("protection", Category::BranchProtection),
            ("teams", Category::Access),
            ("environment", Category::Environments),
            ("integration", Category::Integrations),
        ] {
            assert_eq!(
                parse(&["--category", alias]).unwrap(),
                [expected],
                "{alias}"
            );
        }
    }

    #[test]
    fn values_can_be_comma_separated_and_repeated() {
        let selected = parse(&["--category", "files,security", "--category", "access"]).unwrap();
        assert_eq!(
            selected,
            [Category::Files, Category::Security, Category::Access]
        );
    }

    #[test]
    fn matching_ignores_case() {
        assert_eq!(
            parse(&["--category", "Branch-Protection"]).unwrap(),
            [Category::BranchProtection]
        );
    }

    #[test]
    fn unknown_names_are_rejected() {
        assert!(parse(&["--category", "nope"]).is_err());
    }

    #[test]
    fn no_flag_selects_nothing_so_the_default_is_all() {
        assert!(parse(&[]).unwrap().is_empty());
    }
}
