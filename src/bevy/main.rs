use bevy_ecs::component::Component;
use bevy_ecs::world::World;
use bevy_reflect::Reflect;
use compact_str::CompactString;

#[derive(Component, Reflect)]
pub struct PhysicalDiskLabel(pub CompactString);

#[allow(unused)]
pub fn main() -> eyre::Result<()> {
    let _world = World::default();
    Ok(())
}
