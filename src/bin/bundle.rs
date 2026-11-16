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
            if let Err(error) = copy_application_resources(&path) {
                eprintln!("Failed to copy application resources: {error}");
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
fn copy_application_resources(executable: &std::path::Path) -> Result<(), String> {
    use std::{ffi::c_void, ptr, slice};
    use windows_sys::{
        Win32::{
            Foundation::{FreeLibrary, HMODULE},
            System::LibraryLoader::*,
            UI::WindowsAndMessaging::{RT_GROUP_ICON, RT_ICON, RT_VERSION},
        },
        core::PCWSTR,
    };

    unsafe extern "system" fn collect_name(
        _module: HMODULE,
        _kind: PCWSTR,
        name: PCWSTR,
        output: isize,
    ) -> i32 {
        unsafe { &mut *(output as *mut Vec<PCWSTR>) }.push(name);
        1
    }

    unsafe extern "system" fn collect_language(
        _module: HMODULE,
        _kind: PCWSTR,
        _name: PCWSTR,
        language: u16,
        output: isize,
    ) -> i32 {
        unsafe { &mut *(output as *mut Vec<u16>) }.push(language);
        1
    }

    fn wide(value: &str) -> Vec<u16> {
        value.encode_utf16().chain(std::iter::once(0)).collect()
    }

    let source_path = executable.with_file_name("kockers_browser.dll");
    unsafe {
        let source = LoadLibraryExW(
            wide(&source_path.to_string_lossy()).as_ptr(),
            ptr::null_mut(),
            LOAD_LIBRARY_AS_DATAFILE,
        );
        if source.is_null() {
            return Err(format!("cannot load {}", source_path.display()));
        }
        let update = BeginUpdateResourceW(wide(&executable.to_string_lossy()).as_ptr(), 0);
        if update.is_null() {
            FreeLibrary(source);
            return Err(format!(
                "cannot open {} for resource update",
                executable.display()
            ));
        }
        let mut result = Ok(());
        for kind in [RT_ICON, RT_GROUP_ICON, RT_VERSION] {
            let mut names = Vec::new();
            if EnumResourceNamesW(
                source,
                kind,
                Some(collect_name),
                &mut names as *mut Vec<PCWSTR> as isize,
            ) == 0
            {
                result = Err("cannot enumerate DLL resources".to_owned());
                break;
            }
            for name in names {
                let mut languages = Vec::new();
                if EnumResourceLanguagesW(
                    source,
                    kind,
                    name,
                    Some(collect_language),
                    &mut languages as *mut Vec<u16> as isize,
                ) == 0
                {
                    result = Err("cannot enumerate resource languages".to_owned());
                    break;
                }
                for language in languages {
                    let resource = FindResourceExW(source, kind, name, language);
                    let size = SizeofResource(source, resource);
                    let loaded = LoadResource(source, resource);
                    let data = LockResource(loaded);
                    if resource.is_null() || loaded.is_null() || data.is_null() || size == 0 {
                        result = Err("cannot read DLL resource data".to_owned());
                        break;
                    }
                    let bytes = slice::from_raw_parts(data as *const u8, size as usize);
                    if UpdateResourceW(
                        update,
                        kind,
                        name,
                        language,
                        bytes.as_ptr() as *const c_void,
                        size,
                    ) == 0
                    {
                        result = Err("cannot write EXE resource data".to_owned());
                        break;
                    }
                }
                if result.is_err() {
                    break;
                }
            }
            if result.is_err() {
                break;
            }
        }
        if EndUpdateResourceW(update, i32::from(result.is_err())) == 0 && result.is_ok() {
            result = Err("cannot commit EXE resources".to_owned());
        }
        FreeLibrary(source);
        result
    }
}

#[cfg(not(target_os = "windows"))]
fn main() {
    eprintln!("The kockers-bundle MVP utility currently supports Windows only.");
}
