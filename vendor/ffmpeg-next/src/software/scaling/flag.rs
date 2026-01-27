use ffi::*;
use libc::c_int;

bitflags! {
    pub struct Flags: c_int {
        const FAST_BILINEAR        = SwsFlags::SWS_FAST_BILINEAR as c_int;
        const BILINEAR             = SwsFlags::SWS_BILINEAR as c_int;
        const BICUBIC              = SwsFlags::SWS_BICUBIC as c_int;
        const X                    = SwsFlags::SWS_X as c_int;
        const POINT                = SwsFlags::SWS_POINT as c_int;
        const AREA                 = SwsFlags::SWS_AREA as c_int;
        const BICUBLIN             = SwsFlags::SWS_BICUBLIN as c_int;
        const GAUSS                = SwsFlags::SWS_GAUSS as c_int;
        const SINC                 = SwsFlags::SWS_SINC as c_int;
        const LANCZOS              = SwsFlags::SWS_LANCZOS as c_int;
        const SPLINE               = SwsFlags::SWS_SPLINE as c_int;
        const SRC_V_CHR_DROP_MASK  = 0;
        const SRC_V_CHR_DROP_SHIFT = 0;
        const PARAM_DEFAULT        = 0;
        const PRINT_INFO           = SwsFlags::SWS_PRINT_INFO as c_int;
        const FULL_CHR_H_INT       = SwsFlags::SWS_FULL_CHR_H_INT as c_int;
        const FULL_CHR_H_INP       = SwsFlags::SWS_FULL_CHR_H_INP as c_int;
        const DIRECT_BGR           = SwsFlags::SWS_DIRECT_BGR as c_int;
        const ACCURATE_RND         = SwsFlags::SWS_ACCURATE_RND as c_int;
        const BITEXACT             = SwsFlags::SWS_BITEXACT as c_int;
        const ERROR_DIFFUSION      = SwsFlags::SWS_ERROR_DIFFUSION as c_int;
    }
}
