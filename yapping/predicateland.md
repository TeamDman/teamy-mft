# Predicate System Architecture

## Overview

The predicate system is an event-driven entity evaluation framework built on Bevy ECS. It enables reactive filtering and validation of entities through composable predicate rules.

## Core Concepts

### Predicate Entity

A predicate is an entity that:
- Has a `Predicate` marker component
- Has exactly one predicate implementation component (e.g., `FileExtensionPredicate`, `StringEndsWithPredicate`)
- Automatically gets `PredicateEvaluationRequests` via component requirements
- Can be ephemeral (auto-cleanup with `DespawnPredicateWhenDone`)

### Evaluation Flow

1. **Request**: Trigger `RequestPredicateEvaluation` event targeting a predicate entity
2. **Queuing**: Event observer adds entities to `PredicateEvaluationRequests.to_evaluate`
3. **Processing**: Predicate-specific evaluate system processes queued entities
4. **Outcomes**: System triggers outcome events for each evaluated entity
5. **Cleanup**: Predicate despawns when queue is empty (if `DespawnPredicateWhenDone`)

### Outcome Events

All outcome events target the **predicate entity** (not the evaluated entity):

```rust
PredicateOutcomeSuccess {
    predicate: Entity,    // #[event_target] - the predicate that evaluated
    evaluated: Entity,    // the entity that was evaluated
}

PredicateOutcomeFailure {
    predicate: Entity,    // evaluation determined this entity fails the rule
    evaluated: Entity,
}

PredicateOutcomeUnknown {
    predicate: Entity,    // evaluation couldn't determine outcome
    evaluated: Entity,    // (entity missing, components missing, etc.)
}
```

**Key Design Decision**: Events target the predicate because:
- Logic focuses on outcomes of a specific predicate
- Enables entity-scoped observers (`.observe()` on predicate entity)
- Observer lifecycle tied to predicate lifecycle
- More modular and composable

### Entity-Scoped Observers

Observers should be attached directly to predicate entities:

```rust
let mut predicate_cmd = commands.spawn((
    Name::new("MFT File Extension Filter"),
    Predicate,
    DespawnPredicateWhenDone,
    FileExtensionPredicate::new("mft"),
));
predicate_cmd.observe(on_mft_predicate_success);  // ← Observer scoped to this predicate
let predicate = predicate_cmd.id();
```

**Benefits**:
- Observer automatically cleaned up when predicate despawns
- No global observer pollution
- Clear ownership and lifecycle management

## Predicate Implementations

### FileExtensionPredicate

Efficiently checks file extensions without string allocation:

```rust
FileExtensionPredicate {
    extension: CompactString,  // "txt", "mft", etc. (no leading dot)
    case_insensitive: bool,
}

// Constructor defaults to case-insensitive
FileExtensionPredicate::new("mft")
```

**Implementation Details**:
- Uses `Path::extension()` → `OsStr::to_str()` → comparison
- Avoids `to_string_lossy()` allocation
- Guard clause emits `PredicateOutcomeUnknown` if entity missing

### StringEndsWithPredicate

String suffix matching (less efficient, use FileExtensionPredicate for extensions):

```rust
StringEndsWithPredicate {
    suffix: CompactString,  // ".txt", "_backup", etc.
}
```

**Note**: Uses `to_string_lossy()` which allocates. Prefer `FileExtensionPredicate` for file extensions.

## Code Style Preferences

### Variable Naming in Evaluate Functions

Use descriptive names with struct shorthand:

```rust
fn evaluate(
    mut predicates: Query<(Entity, &SomePredicate, &mut PredicateEvaluationRequests), With<Predicate>>,
    to_evaluate: Query<&SomeComponent>,
    mut commands: Commands,
) {
    for (predicate, rule, mut requests) in predicates.iter_mut() {
        // predicate: Entity ID of the predicate
        // rule: The predicate component with evaluation logic
        // requests: The evaluation queue
        
        for evaluated in requests.to_evaluate.drain() {
            // evaluated: Entity being evaluated
            
            // Enables struct shorthand:
            commands.trigger(PredicateOutcomeSuccess { predicate, evaluated });
        }
    }
}
```

### Guard Clauses

Prefer early returns/continues to reduce nesting:

```rust
// ✅ Good - guard clause
for evaluated in requests.to_evaluate.drain() {
    let Ok(component) = to_evaluate.get(evaluated) else {
        commands.trigger(PredicateOutcomeUnknown { predicate, evaluated });
        continue;
    };
    
    // Main logic at lower indentation
    if component.matches() {
        commands.trigger(PredicateOutcomeSuccess { predicate, evaluated });
    }
}

// ❌ Avoid - nested if statements
for evaluated in requests.to_evaluate.drain() {
    if let Ok(component) = to_evaluate.get(evaluated) {
        if component.matches() {
            commands.trigger(PredicateOutcomeSuccess { predicate, evaluated });
        }
    } else {
        commands.trigger(PredicateOutcomeUnknown { predicate, evaluated });
    }
}
```

### Unknown vs Failure

- **Unknown**: Entity not found, required components missing, can't evaluate
- **Failure**: Successfully evaluated but didn't match the rule

```rust
// Guard clause → Unknown
let Ok(path_holder) = to_evaluate.get(evaluated) else {
    commands.trigger(PredicateOutcomeUnknown { predicate, evaluated });
    continue;
};

// Evaluated rule → Success or Failure
if rule.matches(&path_holder) {
    commands.trigger(PredicateOutcomeSuccess { predicate, evaluated });
} else {
    commands.trigger(PredicateOutcomeFailure { predicate, evaluated });
}
```

## Real-World Example: MFT File Loading

The MFT file plugin demonstrates the predicate pattern:

```rust
pub fn on_sync_dir_child_discovered(
    new_children: Query<&Children, (Changed<Children>, With<SyncDirectory>)>,
    headless: Option<Res<Headless>>,
    testing: Option<Res<Testing>>,
    mut commands: Commands,
) {
    if headless.is_some() || testing.is_some() {
        return;
    }
    
    for children in &new_children {
        let child_entities: Vec<Entity> = children.iter().collect();
        if child_entities.is_empty() {
            continue;
        }
        
        // Spawn ephemeral predicate with observer
        let mut predicate_cmd = commands.spawn((
            Name::new("MFT File Extension Filter"),
            Predicate,
            DespawnPredicateWhenDone,
            FileExtensionPredicate::new("mft"),
        ));
        predicate_cmd.observe(on_mft_predicate_success);
        let predicate = predicate_cmd.id();
        
        // Request evaluation
        commands.trigger(RequestPredicateEvaluation {
            predicate,
            to_evaluate: child_entities.into_iter().collect(),
        });
    }
}

fn on_mft_predicate_success(
    trigger: On<PredicateOutcomeSuccess>,
    path_holders: Query<&PathBufHolder>,
    mut commands: Commands,
    mut messages: ResMut<Messages<MftFileMessage>>,
) {
    let evaluated = trigger.event().evaluated;
    
    if let Ok(path_holder) = path_holders.get(evaluated) {
        let path = path_holder.to_path_buf();
        if path.is_file() {
            commands.entity(evaluated).insert(MftFileNeedsLoading);
            messages.write(MftFileMessage::LoadFromPath(path));
        }
    }
}
```

**Flow**:
1. Sync directory discovers children
2. Spawn ephemeral `.mft` extension filter predicate
3. Attach observer to handle successful matches
4. Request evaluation of all children
5. Observer marks matching files for loading
6. Predicate auto-despawns when queue empty

## Testing Resources

The `Testing` resource disables certain behaviors during tests:

```rust
#[derive(Resource, Debug, Clone, Reflect, Default)]
pub struct Testing;
```

Used to prevent side effects like auto-loading MFT files when running test scenarios.

## Alphabetization Preference

Module declarations should be alphabetized:

```rust
pub mod predicate;
pub mod predicate_file_extension;
pub mod predicate_path_exists;
pub mod predicate_string_ends_with;
```

## Performance Considerations

### Filesystem Metadata

`path.is_file()` and `fs::metadata()` make the **same system call** (`stat`/`GetFileAttributesW`):

```rust
// ❌ Only gets file type, throws away other metadata
if path.is_file() { }

// ✅ Same system call, but you get everything
if let Ok(metadata) = path.metadata() {
    if metadata.is_file() {
        // Also available:
        // - metadata.len() (size)
        // - metadata.modified() (mtime)
        // - metadata.created() (ctime/birthtime)
        // - metadata.permissions()
    }
}
```

**Recommendation**: Use `metadata()` instead of `is_file()` if you might need file size, timestamps, or permissions. It's the same cost but gives you more data.

## Testing Strategy

Use resource-based state tracking in tests:

```rust
#[derive(Resource, Default)]
struct TestResults {
    txt_success: HashSet<Entity>,
    txt_failure: HashSet<Entity>,
}

app.init_resource::<TestResults>();

app.add_observer(move |trigger: On<PredicateOutcomeSuccess>, mut results: ResMut<TestResults>| {
    results.txt_success.insert(trigger.event().evaluated);
});
```

This avoids complex query-based state inspection and provides clear test assertions.

## Future Considerations

### Potential Predicate Types

- `PathExistsPredicate` - Check if path exists on disk
- `FileSizePredicate` - File size range checks
- `ModifiedTimePredicate` - Timestamp-based filtering
- `CompositePredicate` - AND/OR combinations of other predicates

### Metadata Component

Could add a component to cache filesystem metadata:

```rust
#[derive(Component)]
struct FileMetadata {
    size: u64,
    modified: SystemTime,
    is_readonly: bool,
}
```

Attach this in the predicate success observer to avoid redundant `stat` calls.
