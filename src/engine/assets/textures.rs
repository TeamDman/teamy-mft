use bevy::asset::AssetPath;
use std::path::Path;
use strum::VariantArray;

#[derive(VariantArray, Clone, Copy, Eq, Debug, PartialEq)]
pub enum MyTexture {
    Icon,
}
impl From<MyTexture> for AssetPath<'static> {
    fn from(value: MyTexture) -> Self {
        AssetPath::from_path(match value {
            MyTexture::Icon => Path::new("textures/icon.png"),
        })
    }
}

#[cfg(test)]
mod test {
    use crate::engine::assets::textures::MyTexture;
    use bevy::asset::AssetPath;
    use std::path::Path;
    use strum::VariantArray;

    #[test]
    fn texture_exists() {
        for font in MyTexture::VARIANTS.iter().cloned() {
            let assets_dir =
                Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("../../assets");
            let asset_path: AssetPath = font.into();
            let path = assets_dir.join(asset_path.path());
            let exists = path.exists();
            assert!(
                exists,
                "Texture {:?} does not exist in the assets dir: {}",
                font,
                path.display()
            );
        }
    }
}
