//! OS eligibility is checked before runtime side effects, including in DllMain.
//! Windows 10 and 11 both report 10.0. Server builds are deliberately excluded.

#[derive(Clone, Copy, Debug, serde::Serialize)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub build: u32,
    pub product_type: u8,
}

pub fn supported(version: Option<Version>) -> bool {
    version
        .is_some_and(|v| v.major == 10 && v.minor == 0 && v.build >= 22000 && v.product_type == 1)
}

pub fn eligible() -> bool {
    supported(current())
}

#[cfg(windows)]
pub fn current() -> Option<Version> {
    // RTL_OSVERSIONINFOEXW, queried directly so the host's compatibility
    // manifest cannot cause GetVersionEx's older-version result.
    #[repr(C)]
    struct Info {
        size: u32,
        major: u32,
        minor: u32,
        build: u32,
        platform: u32,
        service_pack: [u16; 128],
        service_pack_major: u16,
        service_pack_minor: u16,
        suite: u16,
        product_type: u8,
        reserved: u8,
    }
    #[link(name = "ntdll")]
    extern "system" {
        fn RtlGetVersion(info: *mut Info) -> i32;
    }
    let mut info: Info = unsafe { std::mem::zeroed() };
    info.size = std::mem::size_of::<Info>() as u32;
    if unsafe { RtlGetVersion(&mut info) } < 0 {
        return None;
    }
    Some(Version {
        major: info.major,
        minor: info.minor,
        build: info.build,
        product_type: info.product_type,
    })
}

#[cfg(not(windows))]
pub fn current() -> Option<Version> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_windows_11_client_builds_are_eligible() {
        let client = Version {
            major: 10,
            minor: 0,
            build: 22000,
            product_type: 1,
        };
        for build in [19045, 21999] {
            assert!(!supported(Some(Version { build, ..client })));
        }
        for build in [22000, 22631, 26100, 28000] {
            assert!(supported(Some(Version { build, ..client })));
        }
        for product_type in [0, 2, 3] {
            assert!(!supported(Some(Version {
                product_type,
                ..client
            })));
        }
        assert!(!supported(Some(Version {
            major: 11,
            ..client
        })));
        assert!(!supported(None));
    }

    #[cfg(windows)]
    #[test]
    fn native_probe_returns_complete_version_information() {
        let version = current().expect("RtlGetVersion");
        assert!(version.major >= 10);
        assert!(version.build > 0);
        assert!((1..=3).contains(&version.product_type));
    }
}
