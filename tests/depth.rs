use std::path::PathBuf;

#[test]
#[ignore]
fn create_deep_folders() -> eyre::Result<()> {
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("target");

    let max_depth = 100;
    let max_length = 8000;
    let mut depth = 0;
    let chars = [
        'a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j', 'k', 'l', 'm', 'n', 'o', 'p', 'q', 'r',
        's', 't', 'u', 'v', 'w', 'x', 'y', 'z',
    ];

    loop {
        // Check if we've reached maximum depth
        if depth >= max_depth {
            println!("Reached maximum depth limit: {max_depth}");
            break;
        }

        // Create directory name by cycling through chars
        let dir_name = format!(
            "{}{}{}",
            chars[depth % 26],
            chars[(depth / 26) % 26],
            chars[(depth / (26 * 26)) % 26]
        );

        path.push(&dir_name);

        // Check if we've reached maximum path length
        if path.to_string_lossy().len() >= max_length {
            println!("Reached maximum path length limit: {max_length}");
            path.pop(); // Remove the last component since we didn't create it
            break;
        }

        println!("Attempting to create: {}", path.to_string_lossy());
        match std::fs::create_dir_all(&path) {
            Ok(()) => {
                depth += 1;
                println!(
                    "Created depth {}: {} (path length: {})",
                    depth,
                    dir_name,
                    path.to_string_lossy().len()
                );
            }
            Err(e) => {
                println!(
                    "Failed at depth {}: {} (path length: {})",
                    depth,
                    dir_name,
                    path.to_string_lossy().len()
                );
                println!("Error: {e}");
                break;
            }
        }
    }

    println!("Maximum depth reached: {depth}");
    println!("Final path length: {}", path.to_string_lossy().len());

    Ok(())
}
