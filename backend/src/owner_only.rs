//! Files and folders only the user may read: 0600 and 0700 on Unix. On Windows, a protected ACL (no ACEs
//! inherited from the parent) that gives the current user full access and no one else any; a folder's ACE is
//! inherited by what is made in it afterwards.

use std::fs::OpenOptions;
use std::io::Result;
use std::path::Path;

/// Makes `path`, a file, readable by the user only.
pub fn file(path: &Path) -> Result<()> {
    #[cfg(windows)]
    return acl::apply(path, false);
    #[cfg(not(windows))]
    set_mode(path, 0o600)
}

/// Makes `path`, a folder, openable by the user only.
pub fn dir(path: &Path) -> Result<()> {
    #[cfg(windows)]
    return acl::apply(path, true);
    #[cfg(not(windows))]
    set_mode(path, 0o700)
}

/// Creates the folder `path` (with its parents when `recursive`), openable by the user only from the start.
pub fn create_dir(path: &Path, recursive: bool) -> Result<()> {
    let mut b = std::fs::DirBuilder::new();
    b.recursive(recursive);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
    // Windows: each folder this makes gets the ACL right after, outermost first, while still empty.
    #[cfg(windows)]
    let made: Vec<&Path> = path.ancestors().take(if recursive { usize::MAX } else { 1 }).take_while(|d| !d.as_os_str().is_empty() && !d.exists()).collect();
    b.create(path)?;
    #[cfg(windows)]
    for d in made.iter().rev() {
        dir(d)?;
    }
    Ok(())
}

/// Options that create a file readable by the user only from its first byte. On Windows, std can't hand
/// CreateFileW an ACL, so the file gets its folder's inherited ACEs: make the folder owner-only first (`dir`).
pub fn options() -> OpenOptions {
    #[allow(unused_mut)]
    let mut o = OpenOptions::new();
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut o, 0o600);
    o
}

/// Makes `path` executable: 0700 on Unix. Windows runs a file by its extension, so there it does nothing.
pub fn executable(path: &Path) -> Result<()> {
    #[cfg(windows)]
    return std::fs::metadata(path).map(|_| ());
    #[cfg(not(windows))]
    set_mode(path, 0o700)
}

#[cfg(unix)]
fn set_mode(path: &Path, mode: u32) -> Result<()> {
    std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(mode))
}

#[cfg(not(any(unix, windows)))]
fn set_mode(path: &Path, _mode: u32) -> Result<()> {
    std::fs::metadata(path).map(|_| ())
}

#[cfg(windows)]
mod acl {
    use std::io::{Error, Result};
    use std::os::windows::ffi::OsStrExt;
    use std::path::Path;
    use std::sync::OnceLock;

    use windows_sys::Win32::Foundation::{CloseHandle, ERROR_INVALID_FUNCTION, ERROR_NOT_SUPPORTED, HANDLE};
    use windows_sys::Win32::Security::{
        ACCESS_ALLOWED_ACE, ACL, ACL_REVISION, AddAccessAllowedAceEx, CONTAINER_INHERIT_ACE, CopySid, DACL_SECURITY_INFORMATION, GetLengthSid,
        GetTokenInformation, InitializeAcl, InitializeSecurityDescriptor, OBJECT_INHERIT_ACE, SE_DACL_PROTECTED, SECURITY_DESCRIPTOR,
        SetFileSecurityW, SetSecurityDescriptorControl, SetSecurityDescriptorDacl, TOKEN_QUERY, TOKEN_USER, TokenUser,
    };
    use windows_sys::Win32::Storage::FileSystem::FILE_ALL_ACCESS;
    use windows_sys::Win32::System::SystemServices::SECURITY_DESCRIPTOR_REVISION;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

    /// An owner-only security descriptor. The descriptor points into `acl`, so both stay boxed where they are, for
    /// the life of the process.
    pub(super) struct Descriptor {
        sd: Box<SECURITY_DESCRIPTOR>,
        _acl: Box<[u32]>,
    }
    // Built once, then only read.
    unsafe impl Send for Descriptor {}
    unsafe impl Sync for Descriptor {}

    static USER: OnceLock<Option<Box<[u32]>>> = OnceLock::new();
    static FILE: OnceLock<Option<Descriptor>> = OnceLock::new();
    static DIR: OnceLock<Option<Descriptor>> = OnceLock::new();

    /// The SID of the user Loft runs as, copied out of the process token (DWORD-aligned, as SIDs must be).
    pub(super) fn user() -> Option<&'static [u32]> {
        USER.get_or_init(|| unsafe {
            let mut token: HANDLE = std::ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return None;
            }
            let mut len = 0u32;
            GetTokenInformation(token, TokenUser, std::ptr::null_mut(), 0, &mut len);
            let mut buf = vec![0u64; (len as usize).div_ceil(8).max(1)];
            let ok = GetTokenInformation(token, TokenUser, buf.as_mut_ptr().cast(), len, &mut len) != 0;
            CloseHandle(token);
            if !ok {
                return None;
            }
            let sid = (*buf.as_ptr().cast::<TOKEN_USER>()).User.Sid;
            let n = GetLengthSid(sid);
            let mut out = vec![0u32; (n as usize).div_ceil(4)].into_boxed_slice();
            (CopySid(n, out.as_mut_ptr().cast(), sid) != 0).then_some(out)
        })
        .as_deref()
    }

    fn build(dir: bool) -> Option<Descriptor> {
        let sid = user()?;
        let sid_ptr = sid.as_ptr() as *mut core::ffi::c_void;
        unsafe {
            let size = size_of::<ACL>() + size_of::<ACCESS_ALLOWED_ACE>() - size_of::<u32>() + GetLengthSid(sid_ptr) as usize;
            let mut acl = vec![0u32; size.div_ceil(4)].into_boxed_slice();
            let pacl = acl.as_mut_ptr().cast::<ACL>();
            let inherit = if dir { OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE } else { 0 };
            if InitializeAcl(pacl, (acl.len() * 4) as u32, ACL_REVISION) == 0 || AddAccessAllowedAceEx(pacl, ACL_REVISION, inherit, FILE_ALL_ACCESS, sid_ptr) == 0 {
                return None;
            }
            let mut sd: Box<SECURITY_DESCRIPTOR> = Box::new(std::mem::zeroed());
            let psd = (&mut *sd as *mut SECURITY_DESCRIPTOR).cast();
            if InitializeSecurityDescriptor(psd, SECURITY_DESCRIPTOR_REVISION) == 0
                || SetSecurityDescriptorDacl(psd, 1, pacl, 0) == 0
                || SetSecurityDescriptorControl(psd, SE_DACL_PROTECTED, SE_DACL_PROTECTED) == 0
            {
                return None;
            }
            Some(Descriptor { sd, _acl: acl })
        }
    }

    fn descriptor(dir: bool) -> Option<&'static Descriptor> {
        if dir { DIR.get_or_init(|| build(true)) } else { FILE.get_or_init(|| build(false)) }.as_ref()
    }

    /// Replaces `path`'s DACL with the owner-only one. SetFileSecurityW sets it on `path` alone: unlike
    /// SetNamedSecurityInfoW it never walks the folder's contents, which for the data folder means every agent's
    /// workspace. A file system without ACLs (FAT, exFAT) has nothing to set, as chmod there.
    pub(super) fn apply(path: &Path, dir: bool) -> Result<()> {
        let d = descriptor(dir).ok_or_else(|| Error::other("couldn't build an owner-only ACL for the current user"))?;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        let psd = (&*d.sd as *const SECURITY_DESCRIPTOR).cast_mut().cast();
        if unsafe { SetFileSecurityW(wide.as_ptr(), DACL_SECURITY_INFORMATION, psd) } != 0 {
            return Ok(());
        }
        let e = Error::last_os_error();
        match e.raw_os_error().map(|c| c as u32) {
            Some(ERROR_NOT_SUPPORTED | ERROR_INVALID_FUNCTION) => std::fs::metadata(path).map(|_| ()),
            _ => Err(e),
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use std::path::Path;

    use windows_sys::Win32::Foundation::{ERROR_SUCCESS, LocalFree};
    use windows_sys::Win32::Security::Authorization::{GetNamedSecurityInfoW, SE_FILE_OBJECT};
    use windows_sys::Win32::Security::{
        ACCESS_ALLOWED_ACE, ACL, ACL_SIZE_INFORMATION, AclSizeInformation, CONTAINER_INHERIT_ACE, DACL_SECURITY_INFORMATION, EqualSid, GetAce,
        GetAclInformation, GetSecurityDescriptorControl, INHERITED_ACE, OBJECT_INHERIT_ACE, PSECURITY_DESCRIPTOR, SE_DACL_PROTECTED,
    };
    use windows_sys::Win32::Storage::FileSystem::FILE_ALL_ACCESS;
    use windows_sys::Win32::System::SystemServices::ACCESS_ALLOWED_ACE_TYPE;

    /// `path`'s DACL as (protected, [(allowed, the current user, mask, flags)]).
    fn dacl(path: &Path) -> (bool, Vec<(bool, bool, u32, u32)>) {
        use std::os::windows::ffi::OsStrExt;
        let wide: Vec<u16> = path.as_os_str().encode_wide().chain([0]).collect();
        let user = super::acl::user().unwrap().as_ptr() as *mut core::ffi::c_void;
        unsafe {
            let (mut pacl, mut psd): (*mut ACL, PSECURITY_DESCRIPTOR) = (std::ptr::null_mut(), std::ptr::null_mut());
            let r = GetNamedSecurityInfoW(wide.as_ptr(), SE_FILE_OBJECT, DACL_SECURITY_INFORMATION, std::ptr::null_mut(), std::ptr::null_mut(), &mut pacl, std::ptr::null_mut(), &mut psd);
            assert_eq!(r, ERROR_SUCCESS, "{}", path.display());
            let (mut control, mut rev) = (0u16, 0u32);
            assert_ne!(GetSecurityDescriptorControl(psd, &mut control, &mut rev), 0);
            assert!(!pacl.is_null(), "a null DACL lets everyone in");
            let mut info: ACL_SIZE_INFORMATION = std::mem::zeroed();
            assert_ne!(GetAclInformation(pacl, (&mut info as *mut ACL_SIZE_INFORMATION).cast(), size_of::<ACL_SIZE_INFORMATION>() as u32, AclSizeInformation), 0);
            let aces = (0..info.AceCount)
                .map(|i| {
                    let mut ace = std::ptr::null_mut();
                    assert_ne!(GetAce(pacl, i, &mut ace), 0);
                    let ace = ace.cast::<ACCESS_ALLOWED_ACE>();
                    let allowed = (*ace).Header.AceType as u32 == ACCESS_ALLOWED_ACE_TYPE;
                    let sid = (&raw mut (*ace).SidStart).cast();
                    (allowed, allowed && EqualSid(sid, user) != 0, (*ace).Mask, (*ace).Header.AceFlags as u32)
                })
                .collect();
            LocalFree(psd);
            (control & SE_DACL_PROTECTED != 0, aces)
        }
    }

    #[test]
    fn files_and_folders_get_an_acl_for_the_current_user_only() {
        let root = crate::testdir::for_this_test("owner_only");
        std::fs::create_dir_all(&root).unwrap();
        let only_me = |flags| (true, vec![(true, true, FILE_ALL_ACCESS, flags)]);
        let inherits = OBJECT_INHERIT_ACE | CONTAINER_INHERIT_ACE;

        // Made before, with the parent's ACEs: tightened.
        let old = root.join("settings.json");
        std::fs::write(&old, "{}").unwrap();
        assert!(dacl(&old).1.iter().any(|a| a.3 & INHERITED_ACE != 0), "a plain file inherits its folder's ACEs");
        super::file(&old).unwrap();
        assert_eq!(dacl(&old), only_me(0));
        assert_eq!(std::fs::read_to_string(&old).unwrap(), "{}", "still readable by the user");

        // Folders: their own ACE is inherited by what is made in them afterwards, and by nothing from above.
        let made = root.join("backups/2026-11-23_120000");
        super::create_dir(&made, true).unwrap();
        assert_eq!(dacl(&root.join("backups")), only_me(inherits));
        assert_eq!(dacl(&made), only_me(inherits));
        let existing = root.join(".loft");
        std::fs::create_dir(&existing).unwrap();
        super::dir(&existing).unwrap();
        assert_eq!(dacl(&existing), only_me(inherits));
        // So a file `options()` creates in one is owner-only from its first byte.
        let inside = existing.join("AuthKey.p8.tmp");
        std::io::Write::write_all(&mut super::options().write(true).create_new(true).open(&inside).unwrap(), b"key").unwrap();
        let (protected, aces) = dacl(&inside);
        assert!(!protected);
        assert_eq!(aces, vec![(true, true, FILE_ALL_ACCESS, INHERITED_ACE)], "only the user's ACE, inherited");
        super::file(&inside).unwrap();
        assert_eq!(dacl(&inside), only_me(0));
    }
}
