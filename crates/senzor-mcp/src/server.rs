//! The MCP tools and resources.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{
    GetPromptRequestParams, GetPromptResponse, Implementation, ListPromptsResult,
    ListResourcesResult, PaginatedRequestParams, ReadResourceRequestParams, ReadResourceResponse,
    ReadResourceResult, Resource, ResourceContents, ServerCapabilities, ServerConfig,
    SubscribeRequestParams, SubscriptionFilter, UnsubscribeRequestParams,
};
use rmcp::service::{RequestContext, SubscriptionContext};
use rmcp::{ErrorData, RoleServer, ServerHandler, tool, tool_handler, tool_router};
use senzor_core::engine::EngineState;
use senzor_core::project::{BrokerSpec, DeviceSpec, TemplateSpec};
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;

use crate::host::{Envelope, Reply, Request, RunAction, settled};
use crate::live::{Live, project};
use crate::output::{
    EditOut, InstanceDetail, InstanceStatusOut, InstanceSummary, Message, MessagesOut, OpenOut,
    PinOut, ProjectOut, RunOut, SaveOut, StatusOut, StepOut, TemplatesOut, problems, templates,
};
use crate::params::{
    CopyBuiltinParams, DEFAULT_MESSAGE_LIMIT, DEFAULT_RUN_WAIT_SECONDS, IdParams, InstanceParams,
    MAX_MESSAGE_LIMIT, MAX_MESSAGE_WAIT_SECONDS, MAX_RUN_WAIT_SECONDS, MessagesParams,
    NewProjectParams, OpenProjectParams, PinParams, RenameParams, RunParams, SaveProjectParams,
    SetParams, SetProjectParams, UnpinParams, UnsetParams, wait,
};
use crate::prompts;
use crate::resources::{REFERENCES, find};
use crate::subscriptions::{Subscriptions, Watched, watch};
use crate::view::{Snapshot, View};

/// How long a tool waits for the host to apply a change.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// How long a tool waits for the engine to catch up with a change before reading its state.
const SYNC_TIMEOUT: Duration = Duration::from_secs(2);
const POLL_INTERVAL: Duration = Duration::from_millis(50);

const INSTRUCTIONS: &str = "Senzor simulates MQTT sensors for testing whatever consumes them.
Terms: a template defines topics, payloads and fields; a device ([[devices]] entry) uses a template \
with a count; each instance is one simulated sensor with its own MQTT connection; a field is a \
published value made by a generator; a pin forces a field of a running instance to a value.
Typical loop: get_status, start, get_messages (keep last_seq and pass it as after_seq to see only \
new messages, wait_seconds to wait for one), pin, kill (the broker publishes the Last Will), restart.
Edits change the project in memory and never restart running instances. A running instance takes \
an edit at once, without reconnecting, unless the edit changes its connection (the broker settings \
or its template's availability); edit results list those instances, which need a restart. \
save_project writes the file. Broker passwords are shown as \
<redacted>; sending <redacted> back keeps the password.
The project-file reference is available as resources (senzor://reference/...). Subscribe to \
senzor://project, senzor://instances or senzor://messages to be told when they change instead of \
polling.";

/// Called after a request is queued, so a host that only looks at requests now and then (the
/// GUI, between frames) looks now.
pub type Wake = Arc<dyn Fn() + Send + Sync>;

/// The Senzor MCP server. Clone it for every connection; all clones talk to the same host.
#[derive(Clone)]
pub struct SenzorServer {
    view: View,
    requests: mpsc::UnboundedSender<Envelope>,
    wake: Wake,
    /// How errors name the host when it does not answer, e.g. "the Senzor window".
    host_name: &'static str,
    timeout: Duration,
    /// This connection's `resources/subscribe` subscriptions; a clone starts with none.
    subscriptions: Subscriptions,
}

impl SenzorServer {
    pub fn new(
        view: View,
        requests: mpsc::UnboundedSender<Envelope>,
        wake: Wake,
        host_name: &'static str,
    ) -> Self {
        Self {
            view,
            requests,
            wake,
            host_name,
            timeout: REQUEST_TIMEOUT,
            subscriptions: Subscriptions::default(),
        }
    }

    /// How long a tool waits for the host to apply a change (default [`REQUEST_TIMEOUT`]).
    pub fn with_request_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Hands `request` to the host and waits for its reply.
    async fn call(&self, request: Request) -> Result<Reply, String> {
        let (reply, answer) = oneshot::channel();
        self.requests
            .send(Envelope { request, reply })
            .map_err(|_| "Senzor is shutting down".to_string())?;
        (self.wake)();
        match tokio::time::timeout(self.timeout, answer).await {
            Ok(Ok(result)) => result,
            Ok(Err(_)) => Err("Senzor is shutting down".into()),
            Err(_) => Err(format!(
                "{} did not respond within {}; the request was cancelled and will not be applied",
                self.host_name,
                humantime::format_duration(self.timeout)
            )),
        }
    }

    /// Polls the engine state until `done` holds or `timeout` passes; returns whether it held.
    async fn poll(&self, timeout: Duration, done: impl Fn(&EngineState) -> bool) -> bool {
        let deadline = Instant::now() + timeout;
        loop {
            if let Ok(observer) = self.view.observer()
                && observer.read(&done)
            {
                return true;
            }
            if Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(POLL_INTERVAL).await;
        }
    }

    /// Waits until the engine has handled the commands a change sent.
    async fn sync(&self, sync: u64) {
        self.poll(SYNC_TIMEOUT, |s| s.commands_handled >= sync)
            .await;
    }

    async fn run(&self, action: RunAction, params: RunParams) -> Result<Json<RunOut>, String> {
        let targets = params.targets()?;
        let wait = wait(
            params.wait_seconds,
            DEFAULT_RUN_WAIT_SECONDS,
            MAX_RUN_WAIT_SECONDS,
        );
        let Reply::Ran {
            action,
            watched,
            sync,
        } = self.call(Request::Run(action, targets)).await?
        else {
            return Err(unexpected());
        };
        self.sync(sync).await;
        let done = self
            .poll(wait, |s| watched.iter().all(|w| settled(action, w, s)))
            .await;
        let instances = self.view.observer()?.read(|s| {
            watched
                .iter()
                .map(|w| InstanceStatusOut {
                    id: w.id.clone(),
                    status: s.instances.get(&w.id).map(|i| (&i.status).into()),
                })
                .collect()
        });
        Ok(Json(RunOut {
            instances,
            timed_out: !done && !wait.is_zero(),
        }))
    }

    async fn edit(&self, request: Request) -> Result<Json<EditOut>, String> {
        let Reply::Edited { changed, sync } = self.call(request).await? else {
            return Err(unexpected());
        };
        self.edit_out(changed, sync).await.map(Json)
    }

    /// What an edit left: the project's problems and the instances it affects.
    async fn edit_out(&self, changed: bool, sync: u64) -> Result<EditOut, String> {
        self.sync(sync).await;
        let snapshot = self.view.get()?;
        let (needs_restart, orphaned) = snapshot.observer.read(|s| {
            let ids = |flag: fn(&senzor_core::engine::InstanceState) -> bool| {
                s.instances
                    .iter()
                    .filter(|(_, i)| flag(i))
                    .map(|(id, _)| id.clone())
                    .collect::<Vec<_>>()
            };
            (ids(|i| i.needs_restart), ids(|i| i.orphaned))
        });
        Ok(EditOut {
            changed,
            problems: problems(&snapshot.validated),
            blocked: blocked(&snapshot),
            needs_restart,
            orphaned,
        })
    }

    async fn pin_change(
        &self,
        request: Request,
        instance: String,
        expect: impl Fn(&EngineState) -> bool,
    ) -> Result<Json<PinOut>, String> {
        let Reply::Pinned { sync } = self.call(request).await? else {
            return Err(unexpected());
        };
        self.sync(sync).await;
        // The instance applies the pin itself, a moment after the engine forwarded it.
        self.poll(SYNC_TIMEOUT, expect).await;
        let pinned = self
            .view
            .observer()?
            .instance(&instance)
            .map(|i| i.pinned.into_iter().collect())
            .unwrap_or_default();
        Ok(Json(PinOut { instance, pinned }))
    }
}

fn unexpected() -> String {
    "internal error: unexpected reply from the host".to_string()
}

fn blocked(snapshot: &Snapshot) -> Vec<String> {
    snapshot.validated.blocked.iter().cloned().collect()
}

fn path_text(path: Option<&std::path::Path>) -> Option<String> {
    path.map(|p| p.display().to_string())
}

#[tool_router]
impl SenzorServer {
    #[tool(
        description = "Project summary (file, unsaved changes, broker, problem count) and every instance with its status, counters, pins and restart/orphan flags.",
        annotations(read_only_hint = true)
    )]
    async fn get_status(&self) -> Result<Json<StatusOut>, String> {
        let snapshot = self.view.get()?;
        let instances = snapshot.observer.read(|s| {
            s.instances
                .iter()
                .map(|(id, state)| InstanceSummary::new(id, state))
                .collect()
        });
        Ok(Json(StatusOut {
            path: path_text(snapshot.path.as_deref()),
            dirty: snapshot.dirty,
            broker_host: snapshot.file.broker.host.clone(),
            broker_port: snapshot.file.broker.port,
            problems: snapshot.validated.errors.len(),
            open_error: snapshot.open_error.clone(),
            instances,
        }))
    }

    #[tool(
        description = "One instance: status, counters, its template, every field it publishes and each field's latest value.",
        annotations(read_only_hint = true)
    )]
    async fn get_instance(
        &self,
        Parameters(params): Parameters<InstanceParams>,
    ) -> Result<Json<InstanceDetail>, String> {
        let snapshot = self.view.get()?;
        let id = params.instance;
        let state = snapshot
            .observer
            .instance(&id)
            .ok_or_else(|| format!("there is no instance `{id}`"))?;
        let config = snapshot.validated.project.instance(&id);
        Ok(Json(InstanceDetail {
            summary: InstanceSummary::new(&id, &state),
            template: config.map(|c| c.template_id.clone()),
            fields: match config {
                Some(config) => config.fields.keys().cloned().collect(),
                None => state.values.keys().cloned().collect(),
            },
            values: state.values,
        }))
    }

    #[tool(
        description = "Published messages, oldest first, with sequence numbers. Pass the previous call's last_seq as after_seq to get only new ones; filter by instance, device or topic text; wait_seconds waits for a matching message.",
        annotations(read_only_hint = true)
    )]
    async fn get_messages(
        &self,
        Parameters(params): Parameters<MessagesParams>,
    ) -> Result<Json<MessagesOut>, String> {
        let after = params.after_seq.unwrap_or(0);
        let limit = params
            .limit
            .unwrap_or(DEFAULT_MESSAGE_LIMIT)
            .clamp(1, MAX_MESSAGE_LIMIT);
        let wait = wait(params.wait_seconds, 0.0, MAX_MESSAGE_WAIT_SECONDS);
        let page = |s: &EngineState| {
            s.log_after(after, limit, |entry| {
                params
                    .instance
                    .as_ref()
                    .is_none_or(|i| &entry.instance == i)
                    && params.device.as_ref().is_none_or(|d| {
                        s.instances
                            .get(&entry.instance)
                            .is_some_and(|i| &i.device == d)
                    })
                    && params
                        .topic_contains
                        .as_ref()
                        .is_none_or(|t| entry.topic.contains(t.as_str()))
            })
        };
        self.poll(wait, |s| !page(s).entries.is_empty()).await;
        let page = self.view.observer()?.read(page);
        Ok(Json(MessagesOut {
            messages: page.entries.iter().map(Message::from).collect(),
            last_seq: page.last_seq,
            truncated: page.truncated,
        }))
    }

    #[tool(
        description = "Start stopped or failed instances (running ones are left alone). Target exactly one of all, devices or instances. Waits until they are online, failed or reconnecting (wait_seconds, default 5)."
    )]
    async fn start(
        &self,
        Parameters(params): Parameters<RunParams>,
    ) -> Result<Json<RunOut>, String> {
        self.run(RunAction::Start, params).await
    }

    #[tool(
        description = "Stop instances gracefully: they publish their offline message and disconnect, so the broker does not publish their Last Will. Waits until they have stopped."
    )]
    async fn stop(
        &self,
        Parameters(params): Parameters<RunParams>,
    ) -> Result<Json<RunOut>, String> {
        self.run(RunAction::Stop, params).await
    }

    #[tool(
        description = "Kill instances: drop their connections without a goodbye, as a device losing power would, so the broker publishes their Last Will. Waits until they have stopped.",
        annotations(destructive_hint = true)
    )]
    async fn kill(
        &self,
        Parameters(params): Parameters<RunParams>,
    ) -> Result<Json<RunOut>, String> {
        self.run(RunAction::Kill, params).await
    }

    #[tool(
        description = "Restart instances with the project's current configuration (stopped ones are started; orphans are only stopped). Waits until they are up again."
    )]
    async fn restart(
        &self,
        Parameters(params): Parameters<RunParams>,
    ) -> Result<Json<RunOut>, String> {
        self.run(RunAction::Restart, params).await
    }

    #[tool(
        description = "Force a field of a running instance to a value; it publishes that value until unpinned."
    )]
    async fn pin(&self, Parameters(params): Parameters<PinParams>) -> Result<Json<PinOut>, String> {
        let (instance, field) = (params.instance.clone(), params.field.clone());
        let request = Request::Pin {
            instance: params.instance,
            field: params.field,
            value: params.value,
        };
        let (i, f) = (instance.clone(), field);
        self.pin_change(request, instance, move |s| {
            s.instances.get(&i).is_some_and(|x| x.pinned.contains(&f))
        })
        .await
    }

    #[tool(
        description = "Let a pinned field (or, without field, every pinned field) of a running instance follow its generator again."
    )]
    async fn unpin(
        &self,
        Parameters(params): Parameters<UnpinParams>,
    ) -> Result<Json<PinOut>, String> {
        let request = Request::Unpin {
            instance: params.instance.clone(),
            field: params.field.clone(),
        };
        let (i, f) = (params.instance.clone(), params.field);
        self.pin_change(request, params.instance, move |s| {
            s.instances.get(&i).is_none_or(|x| match &f {
                Some(f) => !x.pinned.contains(f),
                None => x.pinned.is_empty(),
            })
        })
        .await
    }

    #[tool(
        description = "The project as TOML (as Save writes it, passwords redacted), its file, unsaved-changes flag, validation problems and blocked devices.",
        annotations(read_only_hint = true)
    )]
    async fn get_project(&self) -> Result<Json<ProjectOut>, String> {
        Ok(Json(project(&self.view.get()?)))
    }

    #[tool(
        description = "Replace the whole project with TOML text (not saved to the file). Text with syntax or schema errors changes nothing; dry_run only reports the problems the text would have."
    )]
    async fn set_project(
        &self,
        Parameters(params): Parameters<SetProjectParams>,
    ) -> Result<Json<EditOut>, String> {
        let dry_run = params.dry_run == Some(true);
        let request = Request::SetProject {
            toml: params.toml,
            dry_run,
        };
        if dry_run {
            let Reply::Checked(validated) = self.call(request).await? else {
                return Err(unexpected());
            };
            return Ok(Json(EditOut {
                changed: false,
                problems: problems(&validated),
                blocked: validated.blocked.iter().cloned().collect(),
                needs_restart: Vec::new(),
                orphaned: Vec::new(),
            }));
        }
        self.edit(request).await
    }

    #[tool(
        description = "Add a device, or replace the device with the same id. Takes the keys of a [[devices]] table (id, template, count, instance_id, params, autostart, interval, fields). A field override sets some of the field's keys, unit or name; with generator it replaces the generator and keeps the unit and name it does not set."
    )]
    async fn put_device(
        &self,
        Parameters(device): Parameters<DeviceSpec>,
    ) -> Result<Json<EditOut>, String> {
        self.edit(Request::PutDevice(device)).await
    }

    #[tool(description = "Rename a device.")]
    async fn rename_device(
        &self,
        Parameters(params): Parameters<RenameParams>,
    ) -> Result<Json<EditOut>, String> {
        self.edit(Request::RenameDevice {
            from: params.from,
            to: params.to,
        })
        .await
    }

    #[tool(description = "Remove a device from the project. Its running instances become orphans.")]
    async fn remove_device(
        &self,
        Parameters(params): Parameters<IdParams>,
    ) -> Result<Json<EditOut>, String> {
        self.edit(Request::RemoveDevice(params.id)).await
    }

    #[tool(
        description = "Add a template to the project, or replace the project template with the same id. Takes the keys of a [[templates]] table (id, interval, qos, retain, availability, messages, fields). Each field is its generator table and may add a unit and a readable name, e.g. {\"generator\": \"uniform\", \"min\": 400, \"max\": 800, \"unit\": \"ppm\", \"name\": \"CO2\"}. Built-in ids are refused."
    )]
    async fn put_template(
        &self,
        Parameters(template): Parameters<TemplateSpec>,
    ) -> Result<Json<EditOut>, String> {
        self.edit(Request::PutTemplate(template)).await
    }

    #[tool(description = "Rename a project template; devices that use it follow.")]
    async fn rename_template(
        &self,
        Parameters(params): Parameters<RenameParams>,
    ) -> Result<Json<EditOut>, String> {
        self.edit(Request::RenameTemplate {
            from: params.from,
            to: params.to,
        })
        .await
    }

    #[tool(
        description = "Remove a project template. Devices that still use it get a validation problem."
    )]
    async fn remove_template(
        &self,
        Parameters(params): Parameters<IdParams>,
    ) -> Result<Json<EditOut>, String> {
        self.edit(Request::RemoveTemplate(params.id)).await
    }

    #[tool(description = "Copy a built-in template into the project under a new id, to change it.")]
    async fn copy_builtin_template(
        &self,
        Parameters(params): Parameters<CopyBuiltinParams>,
    ) -> Result<Json<EditOut>, String> {
        self.edit(Request::CopyBuiltin {
            builtin: params.builtin,
            new_id: params.new_id,
        })
        .await
    }

    #[tool(
        description = "Replace the broker settings. Takes the keys of the [broker] table (host, port, client_id_prefix, keep_alive, mqtt_version, session_expiry, username, password, password_env, tls). Omitted keys take their defaults: mqtt_version becomes \"5\", session_expiry 0s, and username, password, password_env and tls are removed. To change one setting, pass every key get_project shows under [broker], with password \"<redacted>\" to keep the stored password. Running instances need a restart to use them."
    )]
    async fn set_broker(
        &self,
        Parameters(broker): Parameters<BrokerSpec>,
    ) -> Result<Json<EditOut>, String> {
        self.edit(Request::SetBroker(broker)).await
    }

    #[tool(
        description = "Change one value of the project (not saved to the file). The path is the project file's keys joined by dots, with devices and templates named by id and messages numbered from 1, e.g. devices.office.count, devices.office.params.room, templates.office-air.interval, templates.office-air.fields.co2.max, templates.office-air.messages.1.payload (or messages[1]), broker.port, simulation.seed; quote a key with a dot in it (params.\"a.b\"). The value is JSON of the type the key takes: a string (durations as text, e.g. \"30s\"), number or boolean, or an object or array for a key that holds a table or list. Missing tables on the way are created, and the number after a list's last entry adds an entry (e.g. a message). A value of the wrong type, an unknown key or a path that names nothing returns an error and changes nothing. Ids change with rename_device and rename_template; built-in templates cannot be changed. One call is one undo step."
    )]
    async fn set(
        &self,
        Parameters(params): Parameters<SetParams>,
    ) -> Result<Json<EditOut>, String> {
        self.edit(Request::Set {
            path: params.path,
            value: params.value,
        })
        .await
    }

    #[tool(
        description = "Remove one key of the project, so its default applies (e.g. devices.office.count goes back to 1, templates.office-air.interval to 10s), or remove a list entry such as a message (templates.office-air.messages.2). The path is written as for set. A key the project file requires, or one that is not set, returns an error and changes nothing. One call is one undo step."
    )]
    async fn unset(
        &self,
        Parameters(params): Parameters<UnsetParams>,
    ) -> Result<Json<EditOut>, String> {
        self.edit(Request::Unset { path: params.path }).await
    }

    #[tool(
        description = "Take back the latest change to the project (not to the file). In the Senzor window this is the window's own undo history (Ctrl+Z), so it also takes back the user's edits; each edit tool call is one step, and opening or creating a project starts a new history. Running instances take the change as they take an edit. Returns what the step changed (broker, simulation, device and template ids), whether undo and redo are possible now, and the project's problems as the edit tools do; an error when there is nothing to undo."
    )]
    async fn undo(&self) -> Result<Json<StepOut>, String> {
        self.step(Request::Undo).await
    }

    #[tool(
        description = "Apply again the change the latest undo took back; possible until the next edit. Returns what it changed, like undo; an error when there is nothing to redo."
    )]
    async fn redo(&self) -> Result<Json<StepOut>, String> {
        self.step(Request::Redo).await
    }

    #[tool(
        description = "Built-in and project templates: interval, topics, fields with their generators, and the devices that use each.",
        annotations(read_only_hint = true)
    )]
    async fn list_templates(&self) -> Result<Json<TemplatesOut>, String> {
        Ok(Json(templates(&self.view.get()?.file)))
    }

    #[tool(
        description = "Start a new, empty project. Refused when there are unsaved changes, unless discard_unsaved is true."
    )]
    async fn new_project(
        &self,
        Parameters(params): Parameters<NewProjectParams>,
    ) -> Result<Json<OpenOut>, String> {
        self.open(Request::NewProject {
            discard_unsaved: params.discard_unsaved == Some(true),
        })
        .await
    }

    #[tool(
        description = "Open a project file; its autostart devices start once the current ones have stopped (autostart = false skips that). Refused when there are unsaved changes, unless discard_unsaved is true."
    )]
    async fn open_project(
        &self,
        Parameters(params): Parameters<OpenProjectParams>,
    ) -> Result<Json<OpenOut>, String> {
        self.open(Request::OpenProject {
            path: PathBuf::from(params.path),
            discard_unsaved: params.discard_unsaved == Some(true),
            autostart: params.autostart != Some(false),
        })
        .await
    }

    #[tool(
        description = "Save the project to its file, or with path save it as that file. An existing other file is only replaced with overwrite = true."
    )]
    async fn save_project(
        &self,
        Parameters(params): Parameters<SaveProjectParams>,
    ) -> Result<Json<SaveOut>, String> {
        let request = Request::SaveProject {
            path: params.path.map(PathBuf::from),
            overwrite: params.overwrite == Some(true),
        };
        let Reply::Saved { path } = self.call(request).await? else {
            return Err(unexpected());
        };
        Ok(Json(SaveOut {
            path: path.display().to_string(),
        }))
    }
}

impl SenzorServer {
    async fn step(&self, request: Request) -> Result<Json<StepOut>, String> {
        let Reply::Stepped {
            changes,
            can_undo,
            can_redo,
            sync,
        } = self.call(request).await?
        else {
            return Err(unexpected());
        };
        Ok(Json(StepOut {
            changes,
            can_undo,
            can_redo,
            edit: self.edit_out(true, sync).await?,
        }))
    }

    async fn open(&self, request: Request) -> Result<Json<OpenOut>, String> {
        let Reply::Opened { sync } = self.call(request).await? else {
            return Err(unexpected());
        };
        self.sync(sync).await;
        let snapshot = self.view.get()?;
        Ok(Json(OpenOut {
            path: path_text(snapshot.path.as_deref()),
            problems: problems(&snapshot.validated),
            blocked: blocked(&snapshot),
            instances: snapshot
                .validated
                .project
                .instances()
                .map(|i| i.id.clone())
                .collect(),
        }))
    }
}

#[tool_handler]
impl ServerHandler for SenzorServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_prompts()
                .enable_resources()
                .enable_resources_subscribe()
                .enable_tools()
                .build(),
        )
        .with_server_info(Implementation::new("senzor", env!("CARGO_PKG_VERSION")))
        .with_instructions(INSTRUCTIONS)
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        let live = Live::ALL.into_iter().map(|live| {
            Resource::new(live.uri(), live.name())
                .with_description(live.description())
                .with_mime_type("application/json")
        });
        let references = REFERENCES.iter().map(|r| {
            Resource::new(r.uri, r.name)
                .with_description(r.description)
                .with_mime_type(r.mime_type)
        });
        Ok(ListResourcesResult::with_all_items(
            live.chain(references).collect(),
        ))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let contents = if let Some(live) = Live::find(&request.uri) {
            let text = live
                .read(&self.view)
                .map_err(|e| ErrorData::internal_error(e, None))?;
            ResourceContents::text(text, live.uri()).with_mime_type("application/json")
        } else {
            let reference = find(&request.uri).ok_or_else(|| no_resource(&request.uri))?;
            ResourceContents::text(reference.text(), reference.uri)
                .with_mime_type(reference.mime_type)
        };
        Ok(ReadResourceResult::new(vec![contents]).into())
    }

    // Clients of protocol versions before 2026-07-28 subscribe with `resources/subscribe`.
    async fn subscribe(
        &self,
        request: SubscribeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        match Live::find(&request.uri) {
            Some(live) => self
                .subscriptions
                .subscribe(live, &self.view, &context.peer),
            // A reference page never changes.
            None if find(&request.uri).is_some() => {}
            None => return Err(no_resource(&request.uri)),
        }
        Ok(())
    }

    async fn unsubscribe(
        &self,
        request: UnsubscribeRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<(), ErrorData> {
        if let Some(live) = Live::find(&request.uri) {
            self.subscriptions.unsubscribe(live);
        }
        Ok(())
    }

    // Clients of protocol 2026-07-28 and later subscribe with `subscriptions/listen`.
    fn accepted_subscription_filter(
        &self,
        requested: &SubscriptionFilter,
    ) -> Option<SubscriptionFilter> {
        let known = requested
            .resource_subscriptions
            .iter()
            .flatten()
            .filter(|uri| Live::find(uri).is_some() || find(uri).is_some());
        Some(
            SubscriptionFilter::builder()
                .resource_subscriptions(known)
                .build(),
        )
    }

    async fn listen(&self, context: SubscriptionContext) -> Result<(), ErrorData> {
        let watched: Watched = context
            .accepted()
            .resource_subscriptions
            .iter()
            .flatten()
            .filter_map(|uri| Live::find(uri))
            .map(|live| (live, live.version(&self.view)))
            .collect();
        // Kept until the subscription ends: dropping it would end `updates`.
        let (_watched, receiver) = tokio::sync::watch::channel(watched);
        let sink = context.sink().clone();
        let view = self.view.clone();
        let updates = watch(
            receiver,
            move |live| live.version(&view),
            move |uri| {
                let sink = sink.clone();
                async move { sink.notify_resource_updated(uri).await.is_ok() }
            },
        );
        tokio::select! {
            () = context.cancelled() => {}
            () = updates => {}
        }
        Ok(())
    }

    async fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListPromptsResult, ErrorData> {
        Ok(ListPromptsResult::with_all_items(prompts::list()))
    }

    async fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<GetPromptResponse, ErrorData> {
        prompts::get(&request.name, request.arguments.as_ref()).map(Into::into)
    }
}

fn no_resource(uri: &str) -> ErrorData {
    ErrorData::resource_not_found(format!("no resource `{uri}`"), None)
}
