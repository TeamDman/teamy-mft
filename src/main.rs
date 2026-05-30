use mimalloc::MiMalloc;

#[cfg(feature = "tracy")]
#[global_allocator]
static GLOBAL: tracing_tracy::client::ProfiledAllocator<MiMalloc> =
    tracing_tracy::client::ProfiledAllocator::new(MiMalloc, 0);

#[cfg(not(feature = "tracy"))]
#[global_allocator]
static GLOBAL: MiMalloc = MiMalloc;

fn main() -> eyre::Result<()> {
    teamy_mft::main()
}
