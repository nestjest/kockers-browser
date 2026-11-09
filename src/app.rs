use crate::{browser, platform};
use cef::*;
use std::{
    cell::RefCell,
    sync::{Arc, Mutex},
};

#[allow(clippy::arc_with_non_send_sync)]
pub fn run(main_args: &MainArgs, command_line: &CommandLine, sandbox_info: *mut u8) -> i32 {
    let process_type = CefString::from("type");
    let is_browser_process = command_line.has_switch(Some(&process_type)) != 1;
    let exit_code = execute_process(Some(main_args), None, sandbox_info);

    if !is_browser_process {
        return exit_code;
    }
    if exit_code != -1 {
        platform::show_fatal_error(&format!(
            "CEF завершил browser process с кодом {exit_code} до инициализации."
        ));
        return exit_code;
    }

    let root_cache_path = std::env::var_os("LOCALAPPDATA")
        .map(|path| {
            std::path::PathBuf::from(path)
                .join("Kockers Browser")
                .join("CEF")
        })
        .and_then(|path| path.into_os_string().into_string().ok());
    let Some(root_cache_path) = root_cache_path else {
        platform::show_fatal_error("Не удалось определить каталог данных LOCALAPPDATA.");
        return 1;
    };

    let controller = Arc::new(Mutex::new(browser::BrowserController::default()));
    let mut app = KockersApp::new(controller);
    let settings = Settings {
        root_cache_path: CefString::from(root_cache_path.as_str()),
        no_sandbox: 0,
        multi_threaded_message_loop: 0,
        external_message_pump: 0,
        windowless_rendering_enabled: 0,
        ..Default::default()
    };

    if initialize(
        Some(main_args),
        Some(&settings),
        Some(&mut app),
        sandbox_info,
    ) != 1
    {
        return 1;
    }

    run_message_loop();
    shutdown();
    0
}

wrap_app! {
    struct KockersApp {
        controller: Arc<Mutex<browser::BrowserController>>,
    }

    impl App {
        fn browser_process_handler(&self) -> Option<BrowserProcessHandler> {
            Some(KockersBrowserProcessHandler::new(
                RefCell::new(None),
                self.controller.clone(),
            ))
        }
    }
}

wrap_browser_process_handler! {
    struct KockersBrowserProcessHandler {
        client: RefCell<Option<Client>>,
        controller: Arc<Mutex<browser::BrowserController>>,
    }

    impl BrowserProcessHandler {
        fn on_already_running_app_relaunch(
            &self,
            _command_line: Option<&mut CommandLine>,
            _current_directory: Option<&CefString>,
        ) -> i32 {
            let window = browser::lock_controller(&self.controller).window();
            if !window.is_null() {
                unsafe { platform::focus_address(window) };
            }
            1
        }

        fn on_context_initialized(&self) {
            debug_assert_ne!(currently_on(ThreadId::UI), 0);

            let client = browser::KockersClient::new(self.controller.clone());
            *self.client.borrow_mut() = Some(client);
            let client = self.client.borrow();
            let Some(client) = client.as_ref().cloned() else {
                platform::show_fatal_error("Не удалось сохранить обработчик CEF.");
                quit_message_loop();
                return;
            };

            match platform::create_main_window(self.controller.clone()) {
                Ok(window) => browser::create_main_browser(window, self.controller.clone(), client),
                Err(error) => {
                    platform::show_fatal_error(&error);
                    quit_message_loop();
                }
            }
        }
    }
}
