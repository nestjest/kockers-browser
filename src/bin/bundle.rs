#[cfg(target_os = "windows")]
fn main() {
    let mut release = false;
    let mut output = None;
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--release" => release = true,
            "-o" | "--output" => {
                let Some(path) = arguments.next() else {
                    eprintln!("Missing path after {argument}.");
                    std::process::exit(2);
                };
                output = Some(path);
            }
            "-h" | "--help" => {
                println!("Usage: kockers-bundle [--release] [-o OUTPUT]");
                return;
            }
            _ => {
                eprintln!("Unknown argument: {argument}");
                std::process::exit(2);
            }
        }
    }

    let output = output.unwrap_or_else(|| "target/bundle".to_owned());
    match cef::build_util::win::build_bundle(
        std::path::Path::new(&output),
        "kockers_browser",
        release,
    ) {
        Ok(path) => {
            if let Err(error) = resources::copy_from_dll(&path.with_extension("dll"), &path) {
                eprintln!("Failed to copy icon and version info into the executable: {error}");
                std::process::exit(1);
            }
            println!("Run the app from {}", path.display());
        }
        Err(error) => {
            eprintln!("Failed to bundle Kockers Browser: {error}");
            std::process::exit(1);
        }
    }
}

#[cfg(target_os = "windows")]
mod resources {
    use std::{ffi::c_void, os::windows::ffi::OsStrExt, path::Path, ptr};

    use windows_sys::Win32::Foundation::{FreeLibrary, GetLastError, HMODULE};
    use windows_sys::Win32::System::LibraryLoader::{
        BeginUpdateResourceW, EndUpdateResourceW, EnumResourceNamesW, FindResourceExW,
        LOAD_LIBRARY_AS_DATAFILE, LOAD_LIBRARY_AS_IMAGE_RESOURCE, LoadLibraryExW, LoadResource,
        LockResource, SizeofResource, UpdateResourceW,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::{RT_GROUP_ICON, RT_ICON, RT_VERSION};
    use windows_sys::core::{BOOL, PCWSTR};

    const LANGUAGE: u16 = 0x0409;
    const BOOTSTRAP_ICON_GROUP: u16 = 32512;

    struct Resource {
        kind: PCWSTR,
        id: u16,
        data: Vec<u8>,
    }

    pub fn copy_from_dll(dll: &Path, exe: &Path) -> Result<(), String> {
        let resources = read(dll)?;
        if !resources.iter().any(|resource| resource.kind == RT_GROUP_ICON) {
            return Err(format!("{} has no icon resources", dll.display()));
        }
        unsafe {
            let update = BeginUpdateResourceW(wide(exe).as_ptr(), 0);
            if update.is_null() {
                return Err(last_error("BeginUpdateResourceW"));
            }
            UpdateResourceW(
                update,
                RT_GROUP_ICON,
                id_name(BOOTSTRAP_ICON_GROUP),
                LANGUAGE,
                ptr::null(),
                0,
            );
            for resource in &resources {
                let written = UpdateResourceW(
                    update,
                    resource.kind,
                    id_name(resource.id),
                    LANGUAGE,
                    resource.data.as_ptr() as *const c_void,
                    resource.data.len() as u32,
                );
                if written == 0 {
                    let error = last_error("UpdateResourceW");
                    EndUpdateResourceW(update, 1);
                    return Err(error);
                }
            }
            if EndUpdateResourceW(update, 0) == 0 {
                return Err(last_error("EndUpdateResourceW"));
            }
        }
        Ok(())
    }

    fn read(dll: &Path) -> Result<Vec<Resource>, String> {
        unsafe {
            let module = LoadLibraryExW(
                wide(dll).as_ptr(),
                ptr::null_mut(),
                LOAD_LIBRARY_AS_DATAFILE | LOAD_LIBRARY_AS_IMAGE_RESOURCE,
            );
            if module.is_null() {
                return Err(last_error("LoadLibraryExW"));
            }
            let mut resources = Vec::new();
            let mut result = Ok(());
            'kinds: for kind in [RT_ICON, RT_GROUP_ICON, RT_VERSION] {
                for id in ids(module, kind) {
                    match data(module, kind, id) {
                        Ok(data) => resources.push(Resource { kind, id, data }),
                        Err(error) => {
                            result = Err(error);
                            break 'kinds;
                        }
                    }
                }
            }
            FreeLibrary(module);
            result.map(|()| resources)
        }
    }

    unsafe fn ids(module: HMODULE, kind: PCWSTR) -> Vec<u16> {
        unsafe extern "system" fn collect(
            _: HMODULE,
            _: PCWSTR,
            name: PCWSTR,
            state: isize,
        ) -> BOOL {
            if (name as usize) >> 16 == 0 {
                unsafe { (*(state as *mut Vec<u16>)).push(name as usize as u16) };
            }
            1
        }
        let mut ids = Vec::new();
        unsafe {
            EnumResourceNamesW(module, kind, Some(collect), &mut ids as *mut Vec<u16> as isize);
        }
        ids
    }

    unsafe fn data(module: HMODULE, kind: PCWSTR, id: u16) -> Result<Vec<u8>, String> {
        unsafe {
            let info = FindResourceExW(module, kind, id_name(id), LANGUAGE);
            if info.is_null() {
                return Err(last_error("FindResourceExW"));
            }
            let size = SizeofResource(module, info) as usize;
            let pointer = LockResource(LoadResource(module, info)) as *const u8;
            if pointer.is_null() || size == 0 {
                return Err(last_error("LoadResource"));
            }
            Ok(std::slice::from_raw_parts(pointer, size).to_vec())
        }
    }

    fn id_name(id: u16) -> PCWSTR {
        id as usize as PCWSTR
    }

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain(Some(0)).collect()
    }

    fn last_error(function: &str) -> String {
        format!("{function} failed with error {}", unsafe { GetLastError() })
    }
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("The kockers-bundle MVP utility currently supports Windows only.");
}
