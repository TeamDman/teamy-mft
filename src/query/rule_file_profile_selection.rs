#[derive(Debug, Clone, Copy)]
pub(super) enum RuleFileProfileSelection<'a> {
    All,
    Default,
    Named(&'a str),
}
