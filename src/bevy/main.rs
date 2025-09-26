use crate::bevy::sync_dir::SyncDirectoryPlugin;
use bevy::prelude::*;
use compact_str::CompactString;

#[derive(Component, Reflect)]
pub struct PhysicalDiskLabel(pub CompactString);

#[allow(unused)]
pub fn main() -> eyre::Result<()> {
    let mut app = App::new();
    app.add_plugins(DefaultPlugins);
    app.add_plugins(SyncDirectoryPlugin);
    app.run();
    Ok(())
}

#[cfg(test)]
mod test {
    use crate::bevy::main::main;

    #[test]
    #[ignore]
    fn it_works() -> eyre::Result<()> {
        main()?;
        Ok(())
    }
}
