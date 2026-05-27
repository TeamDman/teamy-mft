use facet::Facet;
use std::num::NonZeroUsize;

#[derive(Default, Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct FacetSafeNonZeroUsizeOption(Option<NonZeroUsize>);

impl FacetSafeNonZeroUsizeOption {
    #[must_use]
    pub fn get(self) -> Option<usize> {
        self.0.map(NonZeroUsize::get)
    }

    #[must_use]
    pub fn is_some_and(self, f: impl FnOnce(usize) -> bool) -> bool {
        self.get().is_some_and(f)
    }
}

impl From<FacetSafeNonZeroUsizeOption> for usize {
    fn from(value: FacetSafeNonZeroUsizeOption) -> Self {
        value.get().unwrap_or(0)
    }
}

impl From<&FacetSafeNonZeroUsizeOption> for usize {
    fn from(value: &FacetSafeNonZeroUsizeOption) -> Self {
        (*value).into()
    }
}

impl From<usize> for FacetSafeNonZeroUsizeOption {
    fn from(value: usize) -> Self {
        Self(NonZeroUsize::new(value))
    }
}

#[allow(
    clippy::unnecessary_wraps,
    reason = "facet proxy conversion callbacks require a Result-returning signature"
)]
unsafe fn facet_safe_non_zero_usize_option_proxy_convert_out(
    target_ptr: facet::PtrConst,
    proxy_ptr: facet::PtrUninit,
) -> Result<facet::PtrMut, String> {
    // SAFETY: `target_ptr` points at a valid wrapper and `proxy_ptr` points at
    // facet-managed storage for a `usize` proxy.
    unsafe {
        let limit = target_ptr.get::<FacetSafeNonZeroUsizeOption>();
        let proxy = usize::from(limit);
        #[allow(
            clippy::cast_ptr_alignment,
            reason = "facet allocates proxy storage with the alignment required by the proxy type"
        )]
        let proxy_mut = proxy_ptr.as_mut_byte_ptr().cast::<usize>();
        proxy_mut.write(proxy);
        Ok(facet::PtrMut::new(proxy_mut.cast::<u8>()))
    }
}

#[allow(
    clippy::unnecessary_wraps,
    reason = "facet proxy conversion callbacks require a Result-returning signature"
)]
unsafe fn facet_safe_non_zero_usize_option_proxy_convert_in(
    proxy_ptr: facet::PtrConst,
    target_ptr: facet::PtrUninit,
) -> Result<facet::PtrMut, String> {
    // SAFETY: `proxy_ptr` points at a valid `usize` proxy and `target_ptr`
    // points at facet-managed storage for the destination wrapper.
    unsafe {
        let proxy = proxy_ptr.read::<usize>();
        #[allow(
            clippy::cast_ptr_alignment,
            reason = "facet allocates target storage with the alignment required by the target type"
        )]
        let target_mut = target_ptr
            .as_mut_byte_ptr()
            .cast::<FacetSafeNonZeroUsizeOption>();
        target_mut.write(FacetSafeNonZeroUsizeOption::from(proxy));
        Ok(facet::PtrMut::new(target_mut.cast::<u8>()))
    }
}

const FACET_SAFE_NON_ZERO_USIZE_OPTION_PROXY: facet::ProxyDef = facet::ProxyDef {
    shape: <usize as Facet>::SHAPE,
    convert_in: facet_safe_non_zero_usize_option_proxy_convert_in,
    convert_out: facet_safe_non_zero_usize_option_proxy_convert_out,
};

// SAFETY: the type is always serialized through a `usize` proxy. The conversion
// preserves all positive `usize` values and maps only `0` to `None`.
unsafe impl Facet<'_> for FacetSafeNonZeroUsizeOption {
    const SHAPE: &'static facet::Shape = &const {
        facet::ShapeBuilder::for_sized::<FacetSafeNonZeroUsizeOption>("FacetSafeNonZeroUsizeOption")
            .module_path("teamy_mft::domain")
            .ty(facet::Type::User(facet::UserType::Opaque))
            .def(facet::Def::Scalar)
            .proxy(&FACET_SAFE_NON_ZERO_USIZE_OPTION_PROXY)
            .build()
    };
}
