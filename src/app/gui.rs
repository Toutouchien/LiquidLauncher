use std::env;
use std::iter::FromIterator;
use std::option::Option::Some;
use std::process::exit;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use directories::ProjectDirs;
use env_logger::Env;
use futures::lock::Mutex;
use log::error;
use path_absolutize::Absolutize;
use sciter::Value;
use tokio::runtime::Runtime;
use tokio::task;
use tokio::task::JoinHandle;

use crate::app::api::LauncherApi;
use crate::LauncherOptions;
use crate::minecraft::launcher::{LauncherData, LaunchingParameter};
use crate::minecraft::prelauncher;
use crate::minecraft::progress::ProgressUpdate;
use crate::minecraft::service::AuthService;

struct RunnerInstance {
    terminator: tokio::sync::oneshot::Sender<()>,
}

struct ConstantLauncherData {
    app_data: ProjectDirs,
    options: LauncherOptions
}

struct EventHandler {
    constant_data: Arc<ConstantLauncherData>,
    runner_instance: Arc<Mutex<Option<RunnerInstance>>>,
    join_handle: Arc<Mutex<Option<JoinHandle<()>>>>,
    async_runtime: Runtime,
}

struct EventFunctions {
    on_output: Value,
    on_progress: Value,
}

fn handle_stdout(value: &Arc<std::sync::Mutex<EventFunctions>>, data: &[u8]) -> Result<()> {
    value.lock().unwrap().on_output.call(None, &make_args!("stdout", String::from_utf8(data.to_vec()).unwrap()), None)?;

    Ok(())
}

fn handle_stderr(value: &Arc<std::sync::Mutex<EventFunctions>>, data: &[u8]) -> Result<()> {
    value.lock().unwrap().on_output.call(None, &make_args!("stderr", String::from_utf8(data.to_vec()).unwrap()), None)?;

    Ok(())
}

fn handle_progress(value: &Arc<std::sync::Mutex<EventFunctions>>, progress_update: ProgressUpdate) -> Result<()> {
    let funcs = value.lock().unwrap();

    match progress_update {
        ProgressUpdate::SetMax(max) => funcs.on_progress.call(None, &make_args!("max", max as i32), None),
        ProgressUpdate::SetProgress(progress) => funcs.on_progress.call(None, &make_args!("progress", progress as i32), None),
        ProgressUpdate::SetLabel(label) => funcs.on_progress.call(None, &make_args!("label", label), None)
    }?;

    Ok(())
}

impl EventHandler {

    // script handler
    fn run_client(&self, build_id: i32, account_data: Value, on_progress: Value, on_output: Value, on_finalization: Value, on_error: Value) -> bool {
        let runner_instance_clone = self.runner_instance.clone();
        let constant_data_clone = self.constant_data.clone();

        let mut runner_instance_content = self.async_runtime.block_on(self.runner_instance.lock());
        let mut join_handle = self.async_runtime.block_on(self.join_handle.lock());

        if runner_instance_content.is_some() {
            return true;
        }

        let (terminator_tx, terminator_rx) = tokio::sync::oneshot::channel();

        let launching_parameter = LaunchingParameter {
            auth_player_name: account_data.get_item("username").as_string().unwrap_or_else(|| "unexpected".to_string()),
            auth_uuid: account_data.get_item("id").as_string().unwrap_or_else(|| "069a79f4-44e9-4726-a5be-fca90e38aaf5".to_string()),
            auth_access_token: account_data.get_item("accessToken").as_string().unwrap_or_else(|| "-".to_string()),
            auth_xuid: "x".to_string(),
            clientid: "x".to_string(),
            user_type: account_data.get_item("type").as_string().unwrap_or_else(|| "legacy".to_string()),
        };

        let app_data = self.constant_data.app_data.clone();

        let jh = self.async_runtime.spawn(async move {
            // todo: cache builds somewhere
            let builds = match LauncherApi::load_all_builds().await {
                Ok(build) => build,
                Err(err) => {
                    on_error.call(None, &make_args!(err.to_string()), None).unwrap();
                    return;
                }
            };
            let build = match builds.iter().find(|x| x.build_id == build_id as u32) {
                Some(build) => build,
                None => {
                    on_error.call(None, &make_args!("unable to find build"), None).unwrap();
                    return;
                }
            };

            if let Err(err) = prelauncher::launch(
                app_data,
                build,
                launching_parameter,
                LauncherData {
                    on_stdout: handle_stdout,
                    on_stderr: handle_stderr,
                    on_progress: handle_progress,
                    data: Box::new(Arc::new(std::sync::Mutex::new(EventFunctions { on_output, on_progress }))),
                    terminator: terminator_rx
                }
            ).await {
                on_error.call(None, &make_args!(err.to_string()), None).unwrap();
            }

            { *runner_instance_clone.lock().await = None; }

            on_finalization.call(None, &make_args!(), None).unwrap();
        });

        *runner_instance_content = Some(RunnerInstance { terminator: terminator_tx });
        *join_handle = Some(jh);

        true
    }

    fn terminate(&self) -> bool {
        let runner_instance = self.runner_instance.clone();
        let join_handle = self.join_handle.clone();

        self.async_runtime.block_on(async move {
            {
                let mut lck = runner_instance.lock().await;

                if let Some(inst) = lck.take() {
                    println!("Sending sigterm");
                    inst.terminator.send(()).unwrap();
                }
            }

            join_handle.lock().await.take().unwrap().await.unwrap();
        });

        true
    }

    // script handler
    fn get_branches(&self, on_response: Value, on_error: Value) -> bool {
        self.async_runtime.spawn(async move {
            match LauncherApi::load_branches().await {
                Ok(branches) => {
                    on_response.call(None, &make_args!(Value::from_iter(branches)), None).unwrap()
                },
                Err(err) => {
                    error!("{:?}", err);

                    on_error.call(None, &make_args!(err.to_string()), None).unwrap()
                }
            };
        });

        true
    }

    fn get_builds(&self, branch: String, on_response: Value, on_error: Value) -> bool {
        self.async_runtime.spawn(async move {
            match LauncherApi::load_builds(branch).await {
                Ok(builds) => {
                    let builds = Value::from_iter(builds.iter().map(|x| {
                        Value::parse(&*serde_json::to_string(x).unwrap()).unwrap()
                    }).collect::<Vec<Value>>());

                    on_response.call(None, &make_args!(builds), None).unwrap()
                },
                Err(err) => {
                    error!("{:?}", err);

                    on_error.call(None, &make_args!(err.to_string()), None).unwrap()
                }
            };
        });

        true
    }

    fn login_mojang(&self, username: String, password: String, on_error: Value, on_response: Value) -> bool {
        self.async_runtime.spawn(async move {
            match AuthService::authenticate(AuthService::MOJANG, username, password).await {
                Ok(acc) => {
                    on_response.call(None, &make_args!(Value::parse(&*serde_json::to_string(&acc).unwrap()).unwrap()), None).unwrap()
                },
                Err(err) => {
                    println!("{:?}", err);

                    on_error.call(None, &make_args!(err.to_string()), None).unwrap()
                }
            };
        });

        true
    }

    fn get_options(&self) -> Value {
        let options = &self.constant_data.options;
        let json_options = options.to_json().unwrap();

        Value::parse(&*json_options).unwrap()
    }

    fn store_options(&self, options: Value) -> bool {
        self.async_runtime.spawn(async move {
            let x = LauncherOptions::from_json(options.to_string());
            println!("{:?}", x);
        });

        true
    }

    fn exit_app(&self) {
        // store app configuration
        LauncherOptions::store(&self.constant_data.options, self.constant_data.app_data.config_dir()).unwrap();

        // exit app
        exit(0);
    } 

}

impl sciter::EventHandler for EventHandler {
    fn get_subscription(&mut self) -> Option<sciter::dom::event::EVENT_GROUPS> {
        Some(sciter::dom::event::default_events() | sciter::dom::event::EVENT_GROUPS::HANDLE_METHOD_CALL)
    }

    // route script calls to our handler
    dispatch_script_call! {
		fn run_client(i32, Value, Value, Value, Value, Value);
		fn terminate();
        fn get_options();
        fn store_options(Value);
		fn get_branches(Value, Value);
        fn get_builds(String, Value, Value);
        fn login_mojang(String, String, Value, Value);
        fn exit_app();
	}
}


/// Runs the GUI and returns when the window is closed.
pub(crate) fn gui_main(app_data: ProjectDirs, options: LauncherOptions) {
    env_logger::Builder::from_env(Env::default().default_filter_or("debug")).init();

    let gui_index = get_path().unwrap();

    let mut frame = sciter::WindowBuilder::main_window()
        .glassy()
        .alpha()
        .fixed()
        .debug()
        .with_size((1000, 600))
        .create();

    frame.event_handler(EventHandler { constant_data: Arc::new(ConstantLauncherData { app_data, options }), runner_instance: Arc::new(Mutex::new(None)), join_handle: Arc::new(Default::default()), async_runtime: Runtime::new().unwrap() });

    frame.load_file(&gui_index);
    frame.run_app();
}

fn get_path() -> Result<String> {
    let mut app_path = env::current_dir()?;
    app_path.push("app");

    let local_index = if app_path.join("public").exists() { // useful for dev env
        app_path.join("public")
    } else {
        app_path
    }.join("index.html");

    if !local_index.exists() {
        return Err(anyhow!("unable to find app index"));
    }

    let absolut_path = local_index.absolutize()?;
    return Ok(format!("file://{}", absolut_path.to_str().unwrap_or("index.html")));
}