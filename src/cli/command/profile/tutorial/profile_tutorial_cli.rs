use crate::query::RULES_FILE_EXTENSION;
use arbitrary::Arbitrary;
use facet::Facet;

#[derive(Facet, Arbitrary, PartialEq, Debug, Default)]
pub struct ProfileTutorialArgs;

impl ProfileTutorialArgs {
    /// Print a concise tutorial that another LLM can ingest to explain Teamy MFT profiles.
    ///
    /// # Errors
    ///
    /// This command does not currently return operational errors.
    pub fn invoke(self) -> eyre::Result<()> {
        println!(
            "\
Teamy MFT profiles are named query-rule overlays built from `{RULES_FILE_EXTENSION}` files discovered using teamy-mft.

The logical default profile only uses files whose names end with `{RULES_FILE_EXTENSION}` and do not contain a profile suffix.
`C:\\a.teamy_mft_rules` contributes to the default profile, but `C:\\a.my-profile-123.teamy_mft_rules` does not.

Queries executed using a profile other than the default profile inherit the rules of the default profile.

Rule syntax supports `DEFAULT RULE IS INCLUDE`, `DEFAULT RULE IS EXCLUDE`, `INCLUDE <pattern>`, `EXCLUDE <pattern>`, and `ORDER <n> INCLUDE|EXCLUDE <pattern>`.

Literal path rules match the whole subtree below that path.

Glob metacharacters such as `*`, `?`, `[]`, and `{{}}` are supported.

Matching rules are evaluated in ascending ORDER, then filename, then line number. The last matching INCLUDE or EXCLUDE wins.

Matching rules with the same ORDER and exact same body are deduplicated before query execution.

If no default rule is declared, unmatched paths are included.

Queries opt into a named profile with `teamy-mft query <needle> --profile <name>`.

`teamy-mft rules add` creates a new rules file in the current working directory by default unless `--rules-file` is provided.

After creating a brand-new rules file, run `teamy-mft sync` so the file path enters the indexed rule discovery set used by queries.

Sample profile name: `my-profile-123`
Sample profile filename: `teamy-mft-rules-20260607-120000.my-profile-123.teamy_mft_rules`

Sample profile file contents:
```text
DEFAULT RULE IS EXCLUDE
INCLUDE C:\\Users\\Teamy\\Documents\\**
INCLUDE C:\\Programming\\Repos\\teamy-mft\\src\\**
INCLUDE C:\\Programming\\Repos\\teamy-mft\\tests\\**
INCLUDE C:\\Programming\\Repos\\teamy-mft\\README.md
```

Interpretation of the sample:
- Start from \"exclude everything\".
- Re-include the documents subtree.
- Re-include the repository source tree, test tree, and README.
- Everything else stays filtered out of query results unless another INCLUDE rule matches it.

Suggested workflow:
1. Run `teamy-mft rules add ...` from the project directory, or create/edit a `*.teamy_mft_rules` file there manually.
2. Put `DEFAULT RULE IS EXCLUDE` at the top when you want an allowlist-style profile.
3. Add a few INCLUDE rules for the directories or files that matter.
4. Run `teamy-mft sync` after creating a new project rules file so query discovery can find its path.
5. Run `teamy-mft query <needle> --profile my-profile-123` to search with that narrowed view.
6. Run `teamy-mft rules list --profile my-profile-123` to inspect the effective rules."
        );

        Ok(())
    }
}
