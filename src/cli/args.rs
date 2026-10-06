use clap::Args;

use crate::reconcile::unified::Category;

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
