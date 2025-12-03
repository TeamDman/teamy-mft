use bevy::asset::AssetPath;
use std::path::Path;
use strum::VariantArray;

#[derive(VariantArray, Clone, Copy, Eq, Debug, PartialEq)]
pub enum MyObject {
    ComputerTower2,
    ComputerTower3,
    GoldenPlatedMagnifyingGlass,
}

impl MyObject {
    pub const fn relative_path(&self) -> &'static str {
        match self {
            MyObject::ComputerTower2 => "objects/computer-tower/computer-tower-2.glb",
            MyObject::ComputerTower3 => "objects/computer-tower/computer-tower-3.glb",
            MyObject::GoldenPlatedMagnifyingGlass => {
                "objects/golden-plated-magnifying-glass/golden-plated-magnifying-glass.glb"
            }
        }
    }
}

impl From<MyObject> for AssetPath<'static> {
    fn from(value: MyObject) -> Self {
        AssetPath::from_path(Path::new(value.relative_path()))
    }
}

#[cfg(test)]
mod test {
    use super::MyObject;
    use std::path::Path;
    use strum::VariantArray;

    #[test]
    fn object_exists() {
        let assets_dir = Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("assets");
        assert!(
            assets_dir.exists(),
            "Assets dir does not exist: {}",
            assets_dir.display()
        );

        for object in MyObject::VARIANTS.iter().copied() {
            let path = assets_dir.join(object.relative_path());
            assert!(
                path.exists(),
                "Object {:?} does not exist in the assets dir: {}",
                object,
                path.display()
            );
        }
    }
}
