use tracing::debug;
use windows::Win32::Foundation::HMODULE;
use windows::Win32::System::LibraryLoader::GetModuleHandleExW;

pub fn get_current_module() -> eyre::Result<HMODULE> {
    unsafe {
        let instance = {
            let mut out = Default::default();
            GetModuleHandleExW(Default::default(), None, &mut out)?;
            debug!(handle = ?out, "Got current module");
            out
        };
        Ok(instance)
    }
}

#[cfg(test)]
mod test {
    #[test]
    fn it_works() -> eyre::Result<()> {
        let module = super::get_current_module()?;
        println!("Current module handle: {:?}", module);
        Ok(())
    }
}
