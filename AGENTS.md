Each subcommand must have its own directory module.
Each subcommand implementation must live in a new `{}_{}_{}_cli.rs` file that `mod.rs` re-exports to ensure fuzzy finders can find the file easily.

Our mantra is "minimize indirection"; do not introduce stupid 3-5 line helper functions that are used once or twice when we can instead just implement some easy, readable procedural code.

If you introduce a function that looks like this

```rust
fn format_optional_u64(value: Option<u64>) -> String {
    value.map_or_else(|| String::from("none"), |value| value.to_string())
}
```

I'm going to kill you. Just unwrap a Cow in-place instead.