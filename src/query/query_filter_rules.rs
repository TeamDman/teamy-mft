use crate::machine::config::published_drive_paths;
use crate::query::ControlFlow;
use crate::query::QueryNeedle;
use crate::query::QueryPlan;
use crate::query::QueryRule;
use crate::query::visit_drive_search_index_rows;
use eyre::Context;
use globset::GlobBuilder;
use globset::GlobMatcher;
use rayon::prelude::*;
use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::debug;
use tracing::info_span;
use tracing::warn;

pub const RULES_FILE_EXTENSION: &str = ".teamy_mft_rules";
pub const DEFAULT_PROFILE_NAME: &str = "default";

pub struct QueryFilterRules {
    matcher_rules: Vec<CompiledRule>,
    files: Vec<DiscoveredRuleFile>,
    default_behavior: DefaultRuleBehavior,
    profile: Option<String>,
}

pub struct DiscoveredRuleFile {
    pub drive_letter: char,
    pub path: PathBuf,
    pub profile: Option<String>,
    pub rules: Vec<RuleLine>,
    compiled_rules: Vec<CompiledRule>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleLine {
    pub line_number: usize,
    pub order: Option<i64>,
    pub directive: RuleDirective,
    pub raw: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleDirective {
    DefaultInclude,
    DefaultExclude,
    Include(String),
    Exclude(String),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DefaultRuleBehavior {
    Include,
    Exclude,
}

#[derive(Clone)]
struct CompiledRule {
    order: i64,
    include: bool,
    line_number: usize,
    raw: String,
    source_path: PathBuf,
    matcher: CompiledPathRule,
}

#[derive(Clone)]
enum CompiledPathRule {
    LiteralSubtree {
        normalized: String,
    },
    Glob {
        normalized_pattern: String,
        matcher: Arc<GlobMatcher>,
    },
}

impl QueryFilterRules {
    /// Build an empty rule set that excludes nothing.
    #[must_use]
    pub fn empty() -> Self {
        Self {
            matcher_rules: Vec::new(),
            files: Vec::new(),
            default_behavior: DefaultRuleBehavior::Include,
            profile: None,
        }
    }

    /// Discover all `.teamy_mft_rules` files from cached search indexes for the given drives.
    ///
    /// # Errors
    ///
    /// Returns an error if a search index cannot be opened or parsed, if a discovered rules
    /// file cannot be read, or if a discovered file contains invalid rule syntax.
    pub fn discover_rule_files_for_drive_letters(
        drive_letters: &[char],
        sync_dir: &Path,
    ) -> eyre::Result<Vec<DiscoveredRuleFile>> {
        let _span = info_span!("discover_query_rule_files").entered();
        let rules_query = QueryPlan::single_rule(QueryRule::EndsWithCaseInsensitive(
            QueryNeedle::new(RULES_FILE_EXTENSION),
        ));
        let results: Vec<eyre::Result<Vec<DiscoveredRuleFile>>> = drive_letters
            .par_iter()
            .map(|drive_letter| {
                discover_rule_files_for_drive(*drive_letter, sync_dir, &rules_query)
            })
            .collect();

        let mut discovered = Vec::new();
        for result in results {
            discovered.extend(result?);
        }
        discovered.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(discovered)
    }

    /// Load one rules file directly from disk, even if it was not discovered through an index.
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read or parsed.
    pub fn load_rule_file(path: &Path) -> eyre::Result<Option<DiscoveredRuleFile>> {
        load_rules_file('?', path)
    }

    /// Load direct child `.teamy_mft_rules` files from one directory without requiring index discovery.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory listing fails or one of the matching files cannot be parsed.
    pub fn load_rule_files_in_directory(path: &Path) -> eyre::Result<Vec<DiscoveredRuleFile>> {
        if !path.is_dir() {
            return Ok(Vec::new());
        }

        let mut files = Vec::new();
        for entry in std::fs::read_dir(path)? {
            let entry = entry?;
            let entry_path = entry.path();
            if !entry_path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.ends_with(RULES_FILE_EXTENSION))
            {
                continue;
            }
            if let Some(file) = load_rules_file('?', &entry_path)? {
                files.push(file);
            }
        }
        files.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(files)
    }

    /// Discover and resolve the effective rule set for one profile.
    ///
    /// # Errors
    ///
    /// Returns an error if discovery fails, the selected profile name is invalid, or the
    /// effective file set contains invalid or contradictory rule syntax.
    pub fn discover_for_drive_letters(
        drive_letters: &[char],
        sync_dir: &Path,
        profile: Option<&str>,
    ) -> eyre::Result<Self> {
        let discovered = Self::discover_rule_files_for_drive_letters(drive_letters, sync_dir)?;
        Self::from_discovered_files(discovered, profile)
    }

    /// Resolve an effective rule set from already-discovered files.
    ///
    /// # Errors
    ///
    /// Returns an error if the selected profile name is invalid or the effective file set
    /// contains invalid or contradictory rule syntax.
    pub fn from_discovered_files(
        files: Vec<DiscoveredRuleFile>,
        profile: Option<&str>,
    ) -> eyre::Result<Self> {
        let profile = normalize_profile_name(profile)?;
        let mut effective_files = files
            .into_iter()
            .filter(|file| file_applies_to_profile(file, profile.as_deref()))
            .collect::<Vec<_>>();
        effective_files.sort_by(|left, right| left.path.cmp(&right.path));
        if let Some(profile_name) = profile.as_deref() {
            if !effective_files
                .iter()
                .any(|file| file.profile.as_deref() == Some(profile_name))
            {
                eyre::bail!(
                    "No {} files were discovered for profile {}. Create or sync a *.{profile_name}{} file first.",
                    RULES_FILE_EXTENSION,
                    profile_name,
                    RULES_FILE_EXTENSION
                );
            }
        }

        let mut matcher_rules = Vec::<CompiledRule>::new();
        let mut default_source: Option<(DefaultRuleBehavior, PathBuf, usize)> = None;
        let mut duplicate_orders = BTreeMap::<i64, Vec<(PathBuf, usize)>>::new();
        let mut seen_matcher_rules = BTreeSet::<(i64, String)>::new();
        let mut added_compiled_rules = BTreeSet::<(i64, String)>::new();

        for file in &effective_files {
            for rule in &file.rules {
                match &rule.directive {
                    RuleDirective::DefaultInclude => match default_source.as_ref() {
                        Some((DefaultRuleBehavior::Exclude, source_path, source_line)) => {
                            eyre::bail!(
                                "{}:{}: conflicting DEFAULT RULE IS EXCLUDE already declared before {}:{} declared DEFAULT RULE IS INCLUDE",
                                source_path.display(),
                                source_line,
                                file.path.display(),
                                rule.line_number
                            );
                        }
                        None => {
                            default_source = Some((
                                DefaultRuleBehavior::Include,
                                file.path.clone(),
                                rule.line_number,
                            ));
                        }
                        Some((DefaultRuleBehavior::Include, _, _)) => {}
                    },
                    RuleDirective::DefaultExclude => match default_source.as_ref() {
                        Some((DefaultRuleBehavior::Include, source_path, source_line)) => {
                            eyre::bail!(
                                "{}:{}: conflicting DEFAULT RULE IS INCLUDE already declared before {}:{} declared DEFAULT RULE IS EXCLUDE",
                                source_path.display(),
                                source_line,
                                file.path.display(),
                                rule.line_number
                            );
                        }
                        None => {
                            default_source = Some((
                                DefaultRuleBehavior::Exclude,
                                file.path.clone(),
                                rule.line_number,
                            ));
                        }
                        Some((DefaultRuleBehavior::Exclude, _, _)) => {}
                    },
                    RuleDirective::Include(_) | RuleDirective::Exclude(_) => {
                        if !seen_matcher_rules
                            .insert((rule.order.unwrap_or(0), rule.render().to_owned()))
                        {
                            continue;
                        }
                        duplicate_orders
                            .entry(rule.order.unwrap_or(0))
                            .or_default()
                            .push((file.path.clone(), rule.line_number));
                    }
                }
            }
            matcher_rules.extend(
                file.compiled_rules
                    .iter()
                    .filter(|rule| added_compiled_rules.insert((rule.order, rule.raw.clone())))
                    .cloned(),
            );
        }

        for (order, entries) in &duplicate_orders {
            if entries.len() < 2 {
                continue;
            }
            let details = entries
                .iter()
                .map(|(path, line)| format!("{}:{}", path.display(), line))
                .collect::<Vec<_>>()
                .join(", ");
            warn!(
                order,
                details, "Multiple INCLUDE/EXCLUDE rules share the same ORDER value"
            );
        }

        matcher_rules.sort_by(|left, right| {
            left.order
                .cmp(&right.order)
                .then_with(|| left.source_path.cmp(&right.source_path))
                .then_with(|| left.line_number.cmp(&right.line_number))
        });

        Ok(Self {
            matcher_rules,
            files: effective_files,
            default_behavior: default_source
                .map_or(DefaultRuleBehavior::Include, |(behavior, _, _)| behavior),
            profile,
        })
    }

    #[must_use]
    pub fn files(&self) -> &[DiscoveredRuleFile] {
        &self.files
    }

    #[must_use]
    pub fn profile_name(&self) -> &str {
        self.profile.as_deref().unwrap_or(DEFAULT_PROFILE_NAME)
    }

    #[must_use]
    pub fn is_filtered_path(&self, path: &Path) -> bool {
        let normalized_path = normalize_candidate_path(path);
        let mut include = match self.default_behavior {
            DefaultRuleBehavior::Include => true,
            DefaultRuleBehavior::Exclude => false,
        };

        for rule in &self.matcher_rules {
            if rule.matcher.is_match(&normalized_path) {
                include = rule.include;
            }
        }

        !include
    }

    #[must_use]
    pub fn matching_files_for_path(&self, path: &Path) -> Vec<&DiscoveredRuleFile> {
        let normalized_path = normalize_candidate_path(path);
        self.files
            .iter()
            .filter(|file| {
                file.compiled_rules
                    .iter()
                    .any(|rule| rule.matcher.is_match(&normalized_path))
            })
            .collect()
    }

    #[must_use]
    pub fn discovered_profile_names(files: &[DiscoveredRuleFile]) -> BTreeSet<String> {
        let mut profiles = files
            .iter()
            .filter_map(|file| file.profile.clone())
            .collect::<BTreeSet<_>>();
        profiles.insert(DEFAULT_PROFILE_NAME.to_owned());
        profiles
    }
}

impl RuleLine {
    #[must_use]
    pub fn render(&self) -> &str {
        &self.raw
    }
}

impl RuleDirective {
    #[must_use]
    pub fn pattern(&self) -> Option<&str> {
        match self {
            Self::Include(pattern) | Self::Exclude(pattern) => Some(pattern),
            Self::DefaultInclude | Self::DefaultExclude => None,
        }
    }
}

impl CompiledPathRule {
    fn is_match(&self, normalized_path: &str) -> bool {
        match self {
            Self::LiteralSubtree { normalized } => {
                normalized_path == normalized
                    || normalized_path
                        .strip_prefix(normalized.as_str())
                        .is_some_and(|suffix| suffix.starts_with('/'))
            }
            Self::Glob { matcher, .. } => matcher.is_match(normalized_path),
        }
    }
}

fn discover_rule_files_for_drive(
    drive_letter: char,
    sync_dir: &Path,
    rules_query: &QueryPlan,
) -> eyre::Result<Vec<DiscoveredRuleFile>> {
    let paths = published_drive_paths(sync_dir, drive_letter);
    if !paths.base_index_path.is_file() {
        return Ok(Vec::new());
    }

    let mut files = Vec::new();
    visit_drive_search_index_rows(drive_letter, sync_dir, rules_query, false, false, |row| {
        let Some(file) = load_rules_file(drive_letter, row.path.as_ref())? else {
            return Ok(ControlFlow::Continue);
        };
        files.push(file);
        Ok(ControlFlow::Continue)
    })?;

    Ok(files)
}

fn load_rules_file(drive_letter: char, path: &Path) -> eyre::Result<Option<DiscoveredRuleFile>> {
    if !path.is_file() {
        debug!(path = %path.display(), "Skipping rules file that no longer exists");
        return Ok(None);
    }

    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::PermissionDenied => {
            warn!(
                drive = %drive_letter,
                path = %path.display(),
                error = %error,
                "Skipping unreadable rules file during query discovery"
            );
            return Ok(None);
        }
        Err(error) => {
            return Err(error)
                .wrap_err_with(|| format!("Failed reading rules file {}", path.display()));
        }
    };

    let profile = detect_profile_from_path(path)?;
    let rules = parse_rule_lines(path, &contents)?;
    let mut compiled_rules = Vec::new();

    for rule in &rules {
        let Some(pattern) = rule.directive.pattern() else {
            continue;
        };
        compiled_rules.push(CompiledRule {
            order: rule.order.unwrap_or(0),
            include: matches!(rule.directive, RuleDirective::Include(_)),
            line_number: rule.line_number,
            raw: rule.render().to_owned(),
            source_path: path.to_path_buf(),
            matcher: compile_path_rule(pattern, path, rule.line_number)?,
        });
    }

    Ok(Some(DiscoveredRuleFile {
        drive_letter,
        path: path.to_path_buf(),
        profile,
        rules,
        compiled_rules,
    }))
}

fn detect_profile_from_path(path: &Path) -> eyre::Result<Option<String>> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| eyre::eyre!("{}: invalid UTF-8 filename", path.display()))?;
    let base_name = file_name
        .strip_suffix(RULES_FILE_EXTENSION)
        .ok_or_else(|| eyre::eyre!("{}: expected {}", path.display(), RULES_FILE_EXTENSION))?;

    Ok(base_name
        .rsplit_once('.')
        .and_then(|(_, profile)| (!profile.is_empty()).then(|| profile.to_owned())))
}

fn parse_rule_lines(path: &Path, contents: &str) -> eyre::Result<Vec<RuleLine>> {
    let mut rules = Vec::new();

    for (index, line) in contents.lines().enumerate() {
        let line_number = index + 1;
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        let Some(rule) = parse_rule_line(trimmed, line_number) else {
            eyre::bail!(
                "{}:{}: unsupported rule syntax: {}",
                path.display(),
                line_number,
                trimmed
            );
        };
        rules.push(rule);
    }

    Ok(rules)
}

fn parse_rule_line(trimmed: &str, line_number: usize) -> Option<RuleLine> {
    if trimmed.eq_ignore_ascii_case("DEFAULT RULE IS INCLUDE") {
        return Some(RuleLine {
            line_number,
            order: None,
            directive: RuleDirective::DefaultInclude,
            raw: trimmed.to_owned(),
        });
    }
    if trimmed.eq_ignore_ascii_case("DEFAULT RULE IS EXCLUDE") {
        return Some(RuleLine {
            line_number,
            order: None,
            directive: RuleDirective::DefaultExclude,
            raw: trimmed.to_owned(),
        });
    }

    if let Some(rest) = strip_case_insensitive_prefix(trimmed, "ORDER ") {
        let (order, tail) = rest.split_once(char::is_whitespace)?;
        let order = order.parse::<i64>().ok()?;
        let tail = tail.trim_start();
        let directive = parse_include_exclude_directive(tail)?;
        return Some(RuleLine {
            line_number,
            order: Some(order),
            directive,
            raw: trimmed.to_owned(),
        });
    }

    Some(RuleLine {
        line_number,
        order: None,
        directive: parse_include_exclude_directive(trimmed)?,
        raw: trimmed.to_owned(),
    })
}

fn parse_include_exclude_directive(trimmed: &str) -> Option<RuleDirective> {
    if let Some(pattern) = strip_case_insensitive_prefix(trimmed, "INCLUDE ") {
        let pattern = pattern.trim();
        if pattern.is_empty() {
            return None;
        }
        return Some(RuleDirective::Include(pattern.to_owned()));
    }

    if let Some(pattern) = strip_case_insensitive_prefix(trimmed, "EXCLUDE ") {
        let pattern = pattern.trim();
        if pattern.is_empty() {
            return None;
        }
        return Some(RuleDirective::Exclude(pattern.to_owned()));
    }

    None
}

fn strip_case_insensitive_prefix<'a>(value: &'a str, prefix: &str) -> Option<&'a str> {
    value
        .get(..prefix.len())
        .filter(|candidate| candidate.eq_ignore_ascii_case(prefix))
        .map(|_| &value[prefix.len()..])
}

fn compile_path_rule(
    pattern: &str,
    path: &Path,
    line_number: usize,
) -> eyre::Result<CompiledPathRule> {
    let normalized_pattern = normalize_pattern(pattern);
    if !contains_glob_metacharacters(pattern) {
        return Ok(CompiledPathRule::LiteralSubtree {
            normalized: normalized_pattern,
        });
    }

    let matcher = GlobBuilder::new(&normalized_pattern)
        .case_insensitive(true)
        .backslash_escape(false)
        .build()
        .wrap_err_with(|| {
            format!(
                "{}:{}: invalid glob pattern: {}",
                path.display(),
                line_number,
                pattern
            )
        })?
        .compile_matcher();

    Ok(CompiledPathRule::Glob {
        normalized_pattern,
        matcher: Arc::new(matcher),
    })
}

fn contains_glob_metacharacters(pattern: &str) -> bool {
    pattern
        .chars()
        .any(|ch| matches!(ch, '*' | '?' | '[' | ']' | '{' | '}'))
}

fn normalize_candidate_path(path: &Path) -> String {
    let raw = path.to_string_lossy();
    let replaced = raw.replace('\\', "/");
    let trimmed = if replaced.len() > 3 {
        replaced.trim_end_matches('/')
    } else {
        replaced.as_str()
    };
    trimmed.to_ascii_lowercase()
}

fn normalize_pattern(pattern: &str) -> String {
    let replaced = pattern.replace('\\', "/");
    let trimmed = if replaced.len() > 3 {
        replaced.trim_end_matches('/')
    } else {
        replaced.as_str()
    };
    trimmed.to_ascii_lowercase()
}

fn file_applies_to_profile(file: &DiscoveredRuleFile, profile: Option<&str>) -> bool {
    match profile {
        None => file.profile.is_none(),
        Some(profile) => file.profile.is_none() || file.profile.as_deref() == Some(profile),
    }
}

pub fn normalize_profile_name(profile: Option<&str>) -> eyre::Result<Option<String>> {
    let Some(profile) = profile.map(str::trim) else {
        return Ok(None);
    };
    if profile.is_empty() || profile.eq_ignore_ascii_case(DEFAULT_PROFILE_NAME) {
        return Ok(None);
    }
    if profile.contains('.') {
        eyre::bail!(
            "profile names must not contain '.' because the final filename segment selects the profile"
        );
    }
    if profile
        .chars()
        .any(|ch| ch.is_control() || matches!(ch, '\\' | '/' | ':'))
    {
        eyre::bail!("profile names must not contain control characters or path separators");
    }
    Ok(Some(profile.to_owned()))
}

/// # Errors
///
/// Returns an error if the path is not UTF-8 or does not end with `.teamy_mft_rules`.
pub fn profile_name_from_rules_path(path: &Path) -> eyre::Result<Option<String>> {
    detect_profile_from_path(path)
}

impl std::fmt::Debug for QueryFilterRules {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("QueryFilterRules")
            .field("files", &self.files)
            .field("default_behavior", &self.default_behavior)
            .field("profile", &self.profile)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for DiscoveredRuleFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DiscoveredRuleFile")
            .field("drive_letter", &self.drive_letter)
            .field("path", &self.path)
            .field("profile", &self.profile)
            .field("rules", &self.rules)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for DefaultRuleBehavior {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Include => f.write_str("Include"),
            Self::Exclude => f.write_str("Exclude"),
        }
    }
}

impl std::fmt::Debug for CompiledPathRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::LiteralSubtree { normalized } => f
                .debug_struct("LiteralSubtree")
                .field("normalized", normalized)
                .finish(),
            Self::Glob {
                normalized_pattern, ..
            } => f
                .debug_struct("Glob")
                .field("normalized_pattern", normalized_pattern)
                .finish_non_exhaustive(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::DEFAULT_PROFILE_NAME;
    use super::QueryFilterRules;
    use super::RULES_FILE_EXTENSION;
    use super::RuleDirective;
    use crate::query::QueryNeedle;
    use crate::query::QueryPlan;
    use crate::query::QueryRule;
    use crate::search_index::format::SearchIndexHeader;
    use crate::search_index::format::SearchIndexPathRow;
    use crate::search_index::search_index_bytes::SearchIndexBytesMut;

    #[test]
    fn global_files_apply_to_default_profile() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let global_path = temp_dir
            .path()
            .join(format!("sample{RULES_FILE_EXTENSION}"));
        std::fs::write(&global_path, "EXCLUDE C:\\private\n").expect("write global rules");

        let rules = QueryFilterRules::from_discovered_files(
            vec![
                super::load_rules_file('C', &global_path)
                    .expect("load global file")
                    .expect("global file exists"),
            ],
            None,
        )
        .expect("build rules");

        assert_eq!(rules.profile_name(), DEFAULT_PROFILE_NAME);
        assert!(rules.is_filtered_path(Path::new(r"C:\private\notes.txt")));
    }

    #[test]
    fn non_default_profiles_include_global_and_specific_rules() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let global_path = temp_dir
            .path()
            .join(format!("sample{RULES_FILE_EXTENSION}"));
        let profile_path = temp_dir
            .path()
            .join(format!("sample.mc-modding{RULES_FILE_EXTENSION}"));
        std::fs::write(&global_path, "EXCLUDE C:\\private\n").expect("write global rules");
        std::fs::write(&profile_path, "INCLUDE C:\\Repos\\Minecraft\\**\\*.java\n")
            .expect("write profile rules");

        let rules = QueryFilterRules::from_discovered_files(
            vec![
                super::load_rules_file('C', &global_path)
                    .expect("load global file")
                    .expect("global file exists"),
                super::load_rules_file('C', &profile_path)
                    .expect("load profile file")
                    .expect("profile file exists"),
            ],
            Some("mc-modding"),
        )
        .expect("build rules");

        assert_eq!(rules.files().len(), 2);
        assert!(rules.is_filtered_path(Path::new(r"C:\private\notes.txt")));
        assert!(!rules.is_filtered_path(Path::new(r"C:\Repos\Minecraft\example\src\Main.java")));
    }

    #[test]
    fn non_default_profile_without_specific_rules_is_rejected() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let global_path = temp_dir
            .path()
            .join(format!("sample{RULES_FILE_EXTENSION}"));
        std::fs::write(&global_path, "EXCLUDE C:\\private\n").expect("write global rules");

        let error = QueryFilterRules::from_discovered_files(
            vec![
                super::load_rules_file('C', &global_path)
                    .expect("load global file")
                    .expect("global file exists"),
            ],
            Some("mc-modding"),
        )
        .expect_err("missing profile-specific rules should fail");

        assert!(
            error
                .to_string()
                .contains("No .teamy_mft_rules files were discovered for profile mc-modding")
        );
    }

    #[test]
    fn default_profile_ignores_profile_specific_files() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let global_path = temp_dir
            .path()
            .join(format!("sample{RULES_FILE_EXTENSION}"));
        let profile_path = temp_dir
            .path()
            .join(format!("sample.mc-modding{RULES_FILE_EXTENSION}"));
        std::fs::write(&global_path, "EXCLUDE C:\\private\n").expect("write global rules");
        std::fs::write(&profile_path, "EXCLUDE C:\\Repos\\Minecraft\n")
            .expect("write profile rules");

        let rules = QueryFilterRules::from_discovered_files(
            vec![
                super::load_rules_file('C', &global_path)
                    .expect("load global file")
                    .expect("global file exists"),
                super::load_rules_file('C', &profile_path)
                    .expect("load profile file")
                    .expect("profile file exists"),
            ],
            Some("default"),
        )
        .expect("build rules");

        assert_eq!(rules.files().len(), 1);
        assert!(rules.is_filtered_path(Path::new(r"C:\private\notes.txt")));
        assert!(!rules.is_filtered_path(Path::new(r"C:\Repos\Minecraft\example\src\Main.java")));
    }

    #[test]
    fn literal_rules_match_descendants() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let rules_path = temp_dir
            .path()
            .join(format!("sample{RULES_FILE_EXTENSION}"));
        std::fs::write(&rules_path, "EXCLUDE C:\\Important\\Taxes\n").expect("write rules file");

        let rules = QueryFilterRules::from_discovered_files(
            vec![
                super::load_rules_file('C', &rules_path)
                    .expect("load rules file")
                    .expect("rules file exists"),
            ],
            None,
        )
        .expect("build rules");

        assert!(rules.is_filtered_path(Path::new(r"C:\Important\Taxes\2026\return.pdf")));
    }

    #[test]
    fn default_rule_is_exclude_hides_unmatched_paths() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let rules_path = temp_dir
            .path()
            .join(format!("sample.mc-modding{RULES_FILE_EXTENSION}"));
        std::fs::write(
            &rules_path,
            "DEFAULT RULE IS EXCLUDE\nINCLUDE C:\\Repos\\Minecraft\\**\\*.java\n",
        )
        .expect("write rules file");

        let rules = QueryFilterRules::from_discovered_files(
            vec![
                super::load_rules_file('C', &rules_path)
                    .expect("load rules file")
                    .expect("rules file exists"),
            ],
            Some("mc-modding"),
        )
        .expect("build rules");

        assert!(rules.is_filtered_path(Path::new(r"C:\notes.txt")));
        assert!(!rules.is_filtered_path(Path::new(r"C:\Repos\Minecraft\example\src\Main.java")));
    }

    #[test]
    fn ordered_rules_use_last_matching_rule() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let rules_path = temp_dir
            .path()
            .join(format!("sample{RULES_FILE_EXTENSION}"));
        std::fs::write(
            &rules_path,
            "ORDER 100 INCLUDE C:\\Programming\\*.java\nORDER 200 EXCLUDE *\n",
        )
        .expect("write rules file");

        let rules = QueryFilterRules::from_discovered_files(
            vec![
                super::load_rules_file('C', &rules_path)
                    .expect("load rules file")
                    .expect("rules file exists"),
            ],
            None,
        )
        .expect("build rules");

        assert!(rules.is_filtered_path(Path::new(r"C:\Programming\Main.java")));
    }

    #[test]
    fn conflicting_defaults_are_rejected() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let path_a = temp_dir.path().join(format!("a{RULES_FILE_EXTENSION}"));
        let path_b = temp_dir.path().join(format!("b{RULES_FILE_EXTENSION}"));
        std::fs::write(&path_a, "DEFAULT RULE IS INCLUDE\n").expect("write path_a");
        std::fs::write(&path_b, "DEFAULT RULE IS EXCLUDE\n").expect("write path_b");

        let error = QueryFilterRules::from_discovered_files(
            vec![
                super::load_rules_file('C', &path_a)
                    .expect("load path_a")
                    .expect("path_a exists"),
                super::load_rules_file('C', &path_b)
                    .expect("load path_b")
                    .expect("path_b exists"),
            ],
            None,
        )
        .expect_err("conflicting defaults should fail");

        assert!(error.to_string().contains(":1: conflicting DEFAULT RULE"));
    }

    #[test]
    fn rules_file_suffix_query_matches_teamy_rule_paths() -> eyre::Result<()> {
        let rows = vec![
            SearchIndexPathRow {
                path: String::from("C:\\repo\\2026-05-23.teamy_mft_rules"),
                has_deleted_entries: false,
            },
            SearchIndexPathRow {
                path: String::from("C:\\repo\\.gitignore"),
                has_deleted_entries: false,
            },
        ];

        let bytes = SearchIndexBytesMut::from_rows(
            SearchIndexHeader::new('C', 123, rows.len() as u64),
            &rows,
        )?
        .into_inner()?;
        let bytes = Box::leak(bytes.into_boxed_slice());
        let parsed = crate::search_index::search_index_bytes::SearchIndexBytes::new(bytes)
            .parse_trusted_for_query()?;
        let plan = QueryPlan::single_rule(QueryRule::EndsWithCaseInsensitive(QueryNeedle::new(
            RULES_FILE_EXTENSION,
        )));

        let indices = plan.query.matching_row_indices(&|rule| {
            crate::query::matching_row_indices_for_rule(&parsed, rule)
        })?;

        assert_eq!(indices, vec![0]);
        Ok(())
    }

    #[test]
    fn discover_rule_files_for_drive_letters_uses_overlay_query_results() -> eyre::Result<()> {
        let temp_dir = tempfile::tempdir()?;
        let published_paths = crate::machine::config::published_drive_paths(temp_dir.path(), 'C');
        let rules_path = temp_dir
            .path()
            .join(format!("sample{RULES_FILE_EXTENSION}"));
        std::fs::write(&rules_path, "EXCLUDE C:\\private\n")?;

        let base_rows = vec![SearchIndexPathRow {
            path: String::from("C:\\repo\\notes.txt"),
            has_deleted_entries: false,
        }];
        let base_bytes = SearchIndexBytesMut::from_rows(
            SearchIndexHeader::new('C', 123, base_rows.len() as u64),
            &base_rows,
        )?
        .into_inner()?;
        std::fs::write(&published_paths.base_index_path, base_bytes)?;

        let overlay_rows = vec![SearchIndexPathRow {
            path: rules_path.display().to_string(),
            has_deleted_entries: false,
        }];
        let overlay_bytes = SearchIndexBytesMut::from_rows(
            SearchIndexHeader::new('C', 123, overlay_rows.len() as u64),
            &overlay_rows,
        )?
        .into_inner()?;
        std::fs::write(&published_paths.overlay_index_path, overlay_bytes)?;

        let discovered =
            QueryFilterRules::discover_rule_files_for_drive_letters(&['C'], temp_dir.path())?;

        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].path, rules_path);
        Ok(())
    }

    #[test]
    fn load_rules_file_detects_profile_from_filename() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let rules_path = temp_dir
            .path()
            .join(format!("sample.mc-modding{RULES_FILE_EXTENSION}"));
        std::fs::write(&rules_path, "EXCLUDE C:\\private\n").expect("write rules file");

        let file = super::load_rules_file('C', &rules_path)
            .expect("load rules file")
            .expect("rules file exists");

        assert_eq!(file.profile.as_deref(), Some("mc-modding"));
    }

    #[test]
    fn profile_names_are_discovered_from_files() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let global_path = temp_dir
            .path()
            .join(format!("sample{RULES_FILE_EXTENSION}"));
        let profile_path = temp_dir
            .path()
            .join(format!("sample.mc-modding{RULES_FILE_EXTENSION}"));
        std::fs::write(&global_path, "EXCLUDE C:\\private\n").expect("write global rules");
        std::fs::write(&profile_path, "EXCLUDE C:\\Repos\\Minecraft\n")
            .expect("write profile rules");

        let files = vec![
            super::load_rules_file('C', &global_path)
                .expect("load global file")
                .expect("global file exists"),
            super::load_rules_file('C', &profile_path)
                .expect("load profile file")
                .expect("profile file exists"),
        ];

        let profiles = QueryFilterRules::discovered_profile_names(&files);

        assert!(profiles.contains(DEFAULT_PROFILE_NAME));
        assert!(profiles.contains("mc-modding"));
    }

    #[test]
    fn parse_ordered_include_rule() {
        let rule = super::parse_rule_line("ORDER 10 INCLUDE C:\\Repos\\**\\*.java", 1)
            .expect("rule should parse");

        assert_eq!(rule.order, Some(10));
        assert_eq!(
            rule.directive,
            RuleDirective::Include(String::from(r"C:\Repos\**\*.java"))
        );
    }

    #[test]
    fn duplicate_identical_rules_are_deduplicated_before_matching() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let path_a = temp_dir.path().join(format!("a{RULES_FILE_EXTENSION}"));
        let path_b = temp_dir.path().join(format!("b{RULES_FILE_EXTENSION}"));
        std::fs::write(&path_a, "ORDER 10 INCLUDE C:\\Repos\\**\\*.java\n").expect("write path_a");
        std::fs::write(&path_b, "ORDER 10 INCLUDE C:\\Repos\\**\\*.java\n").expect("write path_b");

        let rules = QueryFilterRules::from_discovered_files(
            vec![
                super::load_rules_file('C', &path_a)
                    .expect("load path_a")
                    .expect("path_a exists"),
                super::load_rules_file('C', &path_b)
                    .expect("load path_b")
                    .expect("path_b exists"),
            ],
            None,
        )
        .expect("build rules");

        assert_eq!(rules.matcher_rules.len(), 1);
    }

    #[test]
    fn load_rule_files_in_directory_only_reads_matching_extension() {
        let temp_dir = tempfile::tempdir().expect("temp dir");
        let rules_path = temp_dir.path().join(format!("a{RULES_FILE_EXTENSION}"));
        let other_path = temp_dir.path().join("b.txt");
        std::fs::write(&rules_path, "INCLUDE C:\\Repos\n").expect("write rules file");
        std::fs::write(&other_path, "ignore me").expect("write other file");

        let files = QueryFilterRules::load_rule_files_in_directory(temp_dir.path())
            .expect("load directory rules");

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, rules_path);
    }

    use std::path::Path;
}
