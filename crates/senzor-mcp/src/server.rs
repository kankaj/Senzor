//! The MCP tools and resources.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use rmcp::handler::server::wrapper::{Json, Parameters};
use rmcp::model::{
    Implementation, ListResourcesResult, PaginatedRequestParams, ReadResourceRequestParams,
    ReadResourceResponse, ReadResourceResult, Resource, ResourceContents, ServerCapabilities,
    ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, tool, tool_handler, tool_router};
use senzor_core::engine::EngineState;
use senzor_core::project::{BrokerSpec, DeviceSpec, TemplateSpec};
use tokio::sync::{mpsc, oneshot};
use tokio::time::Instant;

use crate::host::{Envelope, Reply, Request, RunAction, settled};
use crate::output::{
    EditOut, InstanceDetail, InstanceStatusOut, InstanceSummary, Message, MessagesOut, OpenOut,
    PinOut, ProjectOut, RunOut, SaveOut, StatusOut, TemplatesOut, problems, templates,
};
use crate::params::{
    CopyBuiltinParams, DEFAULT_MESSAGE_LIMIT, DEFAULT_RUN_WAIT_SECONDS, IdParams, InstanceParams,
    MAX_MESSAGE_LIMIT, MAX_MESSAGE_WAIT_SECONDS, MAX_RUN_WAIT_SECONDS, MessagesParams,
    NewProjectParams, OpenProjectParams, PinParams, RenameParams, RunParams, SaveProjectParams,
    SetProjectParams, UnpinParams, wait,
};
use crate::redact::redacted;
use crate::resources::{REFERENCES, find};
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
Edits change the project in memory and never restart running instances: their results list the \
instances that need a restart. save_project writes the file. Broker passwords are shown as \
<redacted>; sending <redacted> back keeps the password.
The project-file reference is available as resources (senzor://reference/...).";

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
        Ok(Json(EditOut {
            changed,
            problems: problems(&snapshot.validated),
            blocked: blocked(&snapshot),
            needs_restart,
            orphaned,
        }))
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
        let snapshot = self.view.get()?;
        Ok(Json(ProjectOut {
            toml: redacted(&snapshot.file).to_toml_string(),
            path: path_text(snapshot.path.as_deref()),
            dirty: snapshot.dirty,
            problems: problems(&snapshot.validated),
            blocked: blocked(&snapshot),
        }))
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
        description = "Add a device, or replace the device with the same id. Takes the keys of a [[devices]] table (id, template, count, instance_id, params, autostart, interval, fields)."
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
        description = "Add a template to the project, or replace the project template with the same id. Takes the keys of a [[templates]] table (id, interval, qos, retain, availability, messages, fields). Built-in ids are refused."
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
        description = "Replace the broker settings. Takes the keys of the [broker] table (host, port, client_id_prefix, keep_alive, username, password, password_env, tls); omitted keys take their defaults. Running instances need a restart to use them."
    )]
    async fn set_broker(
        &self,
        Parameters(broker): Parameters<BrokerSpec>,
    ) -> Result<Json<EditOut>, String> {
        self.edit(Request::SetBroker(broker)).await
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
                .enable_tools()
                .enable_resources()
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
        let resources = REFERENCES
            .iter()
            .map(|r| {
                Resource::new(r.uri, r.name)
                    .with_description(r.description)
                    .with_mime_type(r.mime_type)
            })
            .collect();
        Ok(ListResourcesResult::with_all_items(resources))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let reference = find(&request.uri).ok_or_else(|| {
            ErrorData::resource_not_found(format!("no resource `{}`", request.uri), None)
        })?;
        let contents = ResourceContents::text(reference.text(), reference.uri)
            .with_mime_type(reference.mime_type);
        Ok(ReadResourceResult::new(vec![contents]).into())
    }
}
