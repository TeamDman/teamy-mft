use bevy::prelude::*;

pub struct PathExistsPredicatePlugin;

impl Plugin for PathExistsPredicatePlugin {
    fn build(&self, _app: &mut App) {
        
    }
}
#[derive(Component, Debug, Reflect)]
pub struct PathExistsPredicate;
