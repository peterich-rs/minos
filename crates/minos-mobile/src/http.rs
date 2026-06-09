//! HTTP client for the backend's `/v1/*` control plane.
//!
//! The mobile client uses this for the pre-WS pairing handshake (POST
//! `/v1/pairing/confirm`), for POST-first query reads such as
//! `/v1/pairing/list-hosts`, and for tearing a specific pair down
//! (`DELETE /v1/pairings/:host_device_id`). The post-pair `Forward` /
//! `Forwarded` and event push traffic still flows over the WebSocket.
//!
//! ADR-0020 removed the iOS device-secret rail; every iOS-originated
//! request authenticates with the bearer alone.

use std::collections::HashSet;
use std::time::Duration;

use http::header::CONTENT_TYPE;
use http::{Method, Request, Response, StatusCode};
use minos_domain::{AgentName, DeviceId, MinosError};
use minos_protocol::{
    AddAgentToGroupRequest, AddGroupMemberRequest, AgentSummary, ApprovalDecisionRequest,
    AssignProjectThreadRequest, AuthRequest, AuthResponse, ConversationAgentMembersResponse,
    ConversationMembersResponse, ConversationReadResponse, ConversationResponse,
    ConversationsResponse, CreateFriendRequestRequest, CreateGroupConversationRequest,
    CreateProjectRequest, CreateProjectResponse, DeleteProjectRequest,
    EnsureDirectConversationRequest, FriendRequestsResponse, FriendsResponse,
    GetThreadLastSeqResponse, HostSummary, ListAgentsResponse, ListChatMessagesRequest,
    ListChatMessagesResponse, ListHostClisRequest, ListHostSkillsCommandRequest,
    ListHostSkillsResponse, ListProjectThreadsParams, ListProjectThreadsResponse,
    ListProjectsResponse, ListThreadsParams, ListThreadsResponse, LogoutRequest, MeHostsResponse,
    MyProfileResponse, ReadThreadParams, ReadThreadResponse, RealtimeWsTicketRequest,
    RealtimeWsTicketResponse, RefreshRequest, RefreshResponse, RegisterAgentRequest,
    RemoveAgentFromGroupRequest, SearchUsersRequest, SearchUsersResponse, SendChatMessageRequest,
    SetMinosIdRequest, UpdateProjectRequest, WriteHostSkillConfigCommandRequest,
    WriteHostSkillConfigResponse,
};
use minos_ui_protocol::{MessageRole, ThreadEndReason, UiEventMessage};
use openwire::{Client, RequestBody, ResponseBody, WireError};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use uuid::Uuid;

use crate::openwire_trace::{logger_interceptor, OpenwireTraceFactory};
use crate::request_trace::{self, RequestTransport};

const HTTP_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Deserialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Debug, Deserialize)]
struct ErrorBody {
    code: String,
    message: String,
}

#[derive(Debug, Deserialize)]
struct WsTicketEnvelope {
    data: WsTicketData,
}

#[derive(Debug, Deserialize)]
struct WsTicketData {
    ticket: String,
    gateway_url: String,
}

#[derive(Debug, Serialize)]
struct PairConfirmRequest<'a> {
    pairing_code: &'a str,
    client_request_id: &'a str,
}

#[derive(Debug, Deserialize)]
struct PairConfirmEnvelope {
    data: PairConfirmData,
}

#[derive(Debug, Deserialize)]
pub struct PairConfirmData {
    pub host_installation_id: String,
    #[allow(dead_code)]
    pub status: String,
    #[allow(dead_code)]
    pub already_confirmed: bool,
}

#[derive(Debug, Deserialize)]
struct ListHostsEnvelope {
    data: ListHostsData,
}

#[derive(Debug, Deserialize)]
struct ListHostsData {
    hosts: Vec<FormalHostSummary>,
}

#[derive(Debug, Deserialize)]
struct FormalHostSummary {
    host_installation_id: String,
    host_display_name: String,
    paired_at_ms: i64,
    linked_via_installation_id: String,
}

#[derive(Debug, Serialize)]
struct SendAgentInputRequest {
    session_id: String,
    text: String,
    client_request_id: String,
}

#[derive(Debug, Deserialize)]
pub struct SendAgentInputResponse {
    pub session_id: String,
    pub turn_id: String,
    pub turn_seq: i64,
}

#[derive(Debug, Serialize)]
struct StopAgentSessionRequest {
    session_id: String,
}

#[derive(Debug, Serialize)]
struct ListAgentSessionsRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    conversation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    project_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    before_started_at_ms: Option<i64>,
    limit: u32,
}

#[derive(Debug, Deserialize)]
struct AgentSessionsResponse {
    sessions: Vec<FormalAgentSessionSummary>,
    next_before_started_at_ms: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct ReadAgentSessionTurnsResponse {
    turns: Vec<AgentTurnMetadata>,
    events: Vec<AgentTurnEvent>,
}

#[derive(Debug, Serialize)]
struct ReadAgentSessionTurnsRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    turn_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    after_turn_seq: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    after_event_seq: Option<i64>,
    limit: u32,
}

#[derive(Debug, Deserialize)]
struct AgentTurnMetadata {
    turn_id: String,
    turn_seq: i64,
    role: String,
    status: String,
    started_at_ms: i64,
    finished_at_ms: Option<i64>,
    summary_text: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AgentTurnEvent {
    turn_id: String,
    kind: String,
    payload: serde_json::Value,
    #[allow(dead_code)]
    created_at_ms: i64,
}

#[derive(Debug, Serialize)]
struct ApprovalRespondRequest<'a> {
    request_id: &'a str,
    decision: &'a serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    client_request_id: Option<&'a str>,
}

#[derive(Debug, Serialize)]
struct LinkProjectAgentSessionRequest<'a> {
    project_id: &'a str,
    session_id: &'a str,
}

#[derive(Debug, Serialize)]
struct ListProjectAgentSessionsRequest<'a> {
    project_id: &'a str,
    limit: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    before_started_at_ms: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct ProjectAgentSessionsResponse {
    sessions: Vec<FormalAgentSessionSummary>,
    #[allow(dead_code)]
    next_before_started_at_ms: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct FormalAgentSessionSummary {
    session_id: String,
    #[allow(dead_code)]
    conversation_id: String,
    #[allow(dead_code)]
    project_id: Option<String>,
    agent_id: Option<String>,
    agent: Option<AgentName>,
    #[allow(dead_code)]
    status: String,
    started_at_ms: i64,
    ended_at_ms: Option<i64>,
    title: Option<String>,
    last_activity_at_ms: i64,
    message_count: u32,
    end_reason: Option<ThreadEndReason>,
}

impl FormalAgentSessionSummary {
    fn into_thread_summary(self) -> Result<minos_protocol::ThreadSummary, MinosError> {
        let session_id = self.session_id;
        let agent = self
            .agent
            .or_else(|| {
                self.agent_id
                    .as_deref()
                    .and_then(agent_name_from_session_agent_id)
            })
            .ok_or_else(|| MinosError::BackendInternal {
                message: format!(
                    "decode FormalAgentSessionSummary: missing agent for {session_id}"
                ),
            })?;

        Ok(minos_protocol::ThreadSummary {
            thread_id: session_id,
            agent,
            title: self.title,
            first_ts_ms: self.started_at_ms,
            last_ts_ms: self.last_activity_at_ms,
            message_count: self.message_count,
            ended_at_ms: self.ended_at_ms,
            end_reason: self.end_reason,
        })
    }
}

pub struct MobileHttpClient {
    client: Client,
    base: String,
    device_id: DeviceId,
    device_name: String,
    device_role: &'static str,
}

impl MobileHttpClient {
    pub fn new(
        backend_ws_url: &str,
        device_id: DeviceId,
        device_name: impl Into<String>,
    ) -> Result<Self, MinosError> {
        let tls_connector =
            crate::tls::build_mobile_tls_connector().map_err(|e| MinosError::BackendInternal {
                message: format!("build mobile TLS connector: {e}"),
            })?;

        let base = http_base(backend_ws_url).ok_or_else(|| MinosError::ConnectFailed {
            url: backend_ws_url.into(),
            message: "cannot derive HTTP base from backend URL".into(),
        })?;
        let client = Client::builder()
            .tls_connector(tls_connector)
            .call_timeout(HTTP_TIMEOUT)
            .application_interceptor(logger_interceptor("mobile_http"))
            .event_listener_factory(OpenwireTraceFactory::new("mobile_http"))
            .build()
            .map_err(|e| MinosError::BackendInternal {
                message: format!("openwire build: {e}"),
            })?;
        Ok(Self {
            client,
            base,
            device_id,
            device_name: device_name.into(),
            device_role: "mobile-client",
        })
    }

    /// Confirm a formal host pairing code with the logged-in account bearer.
    pub async fn pair_confirm(
        &self,
        pairing_code: &str,
        access_token: &str,
    ) -> Result<PairConfirmData, MinosError> {
        let path = "/v1/pairing/confirm";
        let url = format!("{}{path}", self.base);
        let trace_id = start_http_trace(
            Method::POST.as_str(),
            path,
            None,
            Some("formal-pair-confirm".into()),
        );
        let req = PairConfirmRequest {
            pairing_code,
            client_request_id: "mobile-pair-confirm",
        };
        let request = self.request_with_json(Method::POST, &url, Some(access_token), &req)?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let envelope: PairConfirmEnvelope =
                decode_success_json(resp, "PairConfirmEnvelope").await?;
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some(format!(
                    "host_installation_id={}",
                    envelope.data.host_installation_id
                )),
                None,
            );
            Ok(envelope.data)
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    /// Tear down a specific account_host_pairings row. The path-bound
    /// `host_device_id` is the Mac to forget; bearer-only auth post
    /// ADR-0020.
    pub async fn delete_pair(
        &self,
        access_token: &str,
        host_device_id: DeviceId,
    ) -> Result<(), MinosError> {
        let path = format!("/v1/pairings/{host_device_id}");
        let url = format!("{}{path}", self.base);
        let trace_id = start_http_trace(Method::DELETE.as_str(), &path, None, None);
        let request = self.request_without_body(Method::DELETE, &url, Some(access_token))?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status == StatusCode::NO_CONTENT || status == StatusCode::NOT_FOUND {
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some("pairing cleared".into()),
                None,
            );
            Ok(())
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    /// List every Mac paired to the caller's account. Bearer-only.
    pub async fn list_paired_hosts(
        &self,
        access_token: &str,
    ) -> Result<MeHostsResponse, MinosError> {
        let path = "/v1/pairing/list-hosts";
        let url = format!("{}{path}", self.base);
        let trace_id = start_http_trace(Method::POST.as_str(), path, None, None);
        let request = self.request_without_body(Method::POST, &url, Some(access_token))?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let envelope: ListHostsEnvelope =
                decode_success_json(resp, "ListHostsEnvelope").await?;
            let body = formal_hosts_to_me_hosts(envelope.data)?;
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some(format!("hosts={}", body.hosts.len())),
                None,
            );
            Ok(body)
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    /// Bearer-only after ADR-0020. Lists the calling account's threads.
    pub async fn list_threads(
        &self,
        access_token: &str,
        params: ListThreadsParams,
    ) -> Result<ListThreadsResponse, MinosError> {
        let path = "/v1/agent-sessions/list";
        let trace_id = start_http_trace(
            Method::POST.as_str(),
            path,
            None,
            Some(format!(
                "limit={} before_ts_ms={:?} agent={:?}",
                params.limit, params.before_ts_ms, params.agent
            )),
        );
        let url = format!("{}{path}", self.base);
        let request_body = ListAgentSessionsRequest {
            conversation_id: None,
            project_id: None,
            before_started_at_ms: params.before_ts_ms,
            limit: params.limit,
        };
        let request =
            self.request_with_json(Method::POST, &url, Some(access_token), &request_body)?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let body: AgentSessionsResponse =
                decode_success_json(resp, "AgentSessionsResponse").await?;
            let mut threads = body
                .sessions
                .into_iter()
                .map(FormalAgentSessionSummary::into_thread_summary)
                .collect::<Result<Vec<_>, _>>()?;
            if let Some(agent) = params.agent {
                threads.retain(|thread| thread.agent == agent);
            }
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some(format!("threads={}", threads.len())),
                None,
            );
            Ok(ListThreadsResponse {
                threads,
                next_before_ts_ms: body.next_before_started_at_ms,
            })
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    pub async fn read_thread(
        &self,
        access_token: &str,
        params: ReadThreadParams,
    ) -> Result<ReadThreadResponse, MinosError> {
        let thread_id = params.thread_id.clone();
        let after_turn_seq = optional_u64_to_i64(params.from_seq, "ReadThreadParams.from_seq")?;
        let turn_limit = params.limit.clamp(1, 200);
        let turns = self
            .read_agent_session_turns(
                access_token,
                ReadAgentSessionTurnsRequest {
                    session_id: Some(thread_id.clone()),
                    turn_id: None,
                    after_turn_seq,
                    after_event_seq: None,
                    limit: turn_limit,
                },
                Some(thread_id.clone()),
                Some(format!(
                    "read_session_turns limit={} after_turn_seq={after_turn_seq:?}",
                    turn_limit
                )),
            )
            .await
            .map_err(|error| map_agent_session_not_found(error, &thread_id))?;

        let next_seq = if turns.turns.len() == usize::try_from(turn_limit).unwrap_or(usize::MAX) {
            turns
                .turns
                .last()
                .and_then(|turn| u64::try_from(turn.turn_seq).ok())
        } else {
            None
        };
        let mut ui_events = Vec::new();
        for turn in turns.turns {
            let events = self
                .read_agent_session_turns(
                    access_token,
                    ReadAgentSessionTurnsRequest {
                        session_id: None,
                        turn_id: Some(turn.turn_id.clone()),
                        after_turn_seq: None,
                        after_event_seq: None,
                        limit: 200,
                    },
                    Some(thread_id.clone()),
                    Some(format!("read_turn_events turn_id={}", turn.turn_id)),
                )
                .await
                .map_err(|error| map_agent_session_not_found(error, &thread_id))?;
            append_turn_ui_events(&mut ui_events, &turn, events.events);
        }
        Ok(ReadThreadResponse {
            ui_events,
            next_seq,
            thread_end_reason: None,
        })
    }

    pub async fn get_thread_last_seq(
        &self,
        access_token: &str,
        thread_id: &str,
    ) -> Result<GetThreadLastSeqResponse, MinosError> {
        let mut after_turn_seq = None;
        let mut last_seq = 0_u64;
        loop {
            let page = self
                .read_agent_session_turns(
                    access_token,
                    ReadAgentSessionTurnsRequest {
                        session_id: Some(thread_id.into()),
                        turn_id: None,
                        after_turn_seq,
                        after_event_seq: None,
                        limit: 200,
                    },
                    Some(thread_id.into()),
                    Some(format!(
                        "get_session_last_turn after_turn_seq={after_turn_seq:?}"
                    )),
                )
                .await
                .map_err(|error| map_agent_session_not_found(error, thread_id))?;
            let Some(last_turn) = page.turns.last() else {
                break;
            };
            let Some(last_turn_seq) = u64::try_from(last_turn.turn_seq).ok() else {
                return Err(MinosError::BackendInternal {
                    message: format!(
                        "decode ReadAgentSessionTurnsResponse.turn_seq: {}",
                        last_turn.turn_seq
                    ),
                });
            };
            last_seq = last_turn_seq;
            if page.turns.len() < 200 {
                break;
            }
            after_turn_seq = Some(last_turn.turn_seq);
        }
        Ok(GetThreadLastSeqResponse { last_seq })
    }

    async fn read_agent_session_turns(
        &self,
        access_token: &str,
        body: ReadAgentSessionTurnsRequest,
        thread_id: Option<String>,
        request_summary: Option<String>,
    ) -> Result<ReadAgentSessionTurnsResponse, MinosError> {
        let path = "/v1/agent-sessions/read-turns";
        let url = format!("{}{path}", self.base);
        let trace_id = start_http_trace(
            Method::POST.as_str(),
            path,
            thread_id.clone(),
            request_summary,
        );
        let request = self.request_with_json(Method::POST, &url, Some(access_token), &body)?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let turns: ReadAgentSessionTurnsResponse =
                decode_success_json(resp, "ReadAgentSessionTurnsResponse").await?;
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some(format!(
                    "turns={} events={}",
                    turns.turns.len(),
                    turns.events.len()
                )),
                thread_id,
            );
            Ok(turns)
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    pub async fn submit_approval_decision(
        &self,
        access_token: &str,
        req: ApprovalDecisionRequest,
    ) -> Result<(), MinosError> {
        let path = "/v1/approvals/respond";
        let url = format!("{}{path}", self.base);
        let trace_id = start_http_trace(
            Method::POST.as_str(),
            path,
            Some(req.thread_id.clone()),
            Some(format!("request_id={}", req.request_id)),
        );
        let request = self.request_with_json(
            Method::POST,
            &url,
            Some(access_token),
            &ApprovalRespondRequest {
                request_id: &req.request_id,
                decision: &req.decision,
                client_request_id: None,
            },
        )?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some("approval decision forwarded".into()),
                Some(req.thread_id),
            );
            Ok(())
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    pub async fn create_project(
        &self,
        access_token: &str,
        req: CreateProjectRequest,
    ) -> Result<CreateProjectResponse, MinosError> {
        let path = "/v1/projects";
        let url = format!("{}{path}", self.base);
        let trace_id = start_http_trace(
            Method::POST.as_str(),
            path,
            None,
            Some(format!("name={} slug={}", req.name, req.workspace_slug)),
        );
        let request = self.request_with_json(Method::POST, &url, Some(access_token), &req)?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let body: CreateProjectResponse =
                decode_success_json(resp, "CreateProjectResponse").await?;
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some(format!("project_id={}", body.project.project_id)),
                None,
            );
            Ok(body)
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    pub async fn list_projects(
        &self,
        access_token: &str,
    ) -> Result<ListProjectsResponse, MinosError> {
        let path = "/v1/projects/query";
        let url = format!("{}{path}", self.base);
        let trace_id = start_http_trace(Method::POST.as_str(), path, None, None);
        let request = self.request_without_body(Method::POST, &url, Some(access_token))?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let body: ListProjectsResponse =
                decode_success_json(resp, "ListProjectsResponse").await?;
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some(format!("projects={}", body.projects.len())),
                None,
            );
            Ok(body)
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    pub async fn update_project(
        &self,
        access_token: &str,
        req: UpdateProjectRequest,
    ) -> Result<(), MinosError> {
        let path = "/v1/projects/update";
        let url = format!("{}{path}", self.base);
        let trace_id = start_http_trace(
            Method::POST.as_str(),
            path,
            None,
            Some(format!("project_id={}", req.project_id)),
        );
        let request = self.request_with_json(Method::POST, &url, Some(access_token), &req)?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            request_trace::finish_success(trace_id, Some(status.as_u16()), None, None);
            Ok(())
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    pub async fn delete_project(
        &self,
        access_token: &str,
        req: DeleteProjectRequest,
    ) -> Result<(), MinosError> {
        let path = "/v1/projects/delete";
        let url = format!("{}{path}", self.base);
        let trace_id = start_http_trace(
            Method::POST.as_str(),
            path,
            None,
            Some(format!("project_id={}", req.project_id)),
        );
        let request = self.request_with_json(Method::POST, &url, Some(access_token), &req)?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            request_trace::finish_success(trace_id, Some(status.as_u16()), None, None);
            Ok(())
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    pub async fn assign_project_thread(
        &self,
        access_token: &str,
        req: AssignProjectThreadRequest,
    ) -> Result<(), MinosError> {
        let path = "/v1/projects/agent-sessions/link";
        let url = format!("{}{path}", self.base);
        let trace_id = start_http_trace(
            Method::POST.as_str(),
            path,
            None,
            Some(format!(
                "project_id={} thread_id={}",
                req.project_id, req.thread_id
            )),
        );
        let request = self.request_with_json(
            Method::POST,
            &url,
            Some(access_token),
            &LinkProjectAgentSessionRequest {
                project_id: &req.project_id,
                session_id: &req.thread_id,
            },
        )?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            request_trace::finish_success(trace_id, Some(status.as_u16()), None, None);
            Ok(())
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    pub async fn list_project_threads(
        &self,
        access_token: &str,
        req: ListProjectThreadsParams,
    ) -> Result<ListProjectThreadsResponse, MinosError> {
        let path = "/v1/projects/agent-sessions/query";
        let url = format!("{}{path}", self.base);
        let trace_id = start_http_trace(
            Method::POST.as_str(),
            path,
            None,
            Some(format!(
                "project_id={} limit={} before_ts_ms={:?}",
                req.project_id, req.limit, req.before_ts_ms
            )),
        );
        let request = self.request_with_json(
            Method::POST,
            &url,
            Some(access_token),
            &ListProjectAgentSessionsRequest {
                project_id: &req.project_id,
                limit: req.limit,
                before_started_at_ms: req.before_ts_ms,
            },
        )?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let body: ProjectAgentSessionsResponse =
                decode_success_json(resp, "ProjectAgentSessionsResponse").await?;
            let threads = body
                .sessions
                .into_iter()
                .map(FormalAgentSessionSummary::into_thread_summary)
                .collect::<Result<Vec<_>, _>>()?;
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some(format!("threads={}", threads.len())),
                None,
            );
            Ok(ListProjectThreadsResponse { threads })
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    pub async fn my_profile(&self, access_token: &str) -> Result<MyProfileResponse, MinosError> {
        let path = "/v1/me/profile/query";
        let url = format!("{}{path}", self.base);
        let trace_id = start_http_trace(Method::POST.as_str(), path, None, None);
        let request = self.request_without_body(Method::POST, &url, Some(access_token))?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let body = decode_success_json(resp, "MyProfileResponse").await?;
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some("profile".into()),
                None,
            );
            Ok(body)
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    pub async fn set_minos_id(
        &self,
        access_token: &str,
        req: SetMinosIdRequest,
    ) -> Result<MyProfileResponse, MinosError> {
        let path = "/v1/me/profile/minos-id";
        let url = format!("{}{path}", self.base);
        let trace_id = start_http_trace(
            Method::POST.as_str(),
            path,
            None,
            Some(req.minos_id.clone()),
        );
        let request = self.request_with_json(Method::POST, &url, Some(access_token), &req)?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let body = decode_success_json(resp, "MyProfileResponse").await?;
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some("minos_id updated".into()),
                None,
            );
            Ok(body)
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    pub async fn search_users(
        &self,
        access_token: &str,
        minos_id: &str,
    ) -> Result<SearchUsersResponse, MinosError> {
        let path = "/v1/users/search/query";
        let url = format!("{}{path}", self.base);
        let trace_id = start_http_trace(Method::POST.as_str(), path, None, Some(minos_id.into()));
        let request = self.request_with_json(
            Method::POST,
            &url,
            Some(access_token),
            &SearchUsersRequest {
                minos_id: minos_id.into(),
            },
        )?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let body: SearchUsersResponse =
                decode_success_json(resp, "SearchUsersResponse").await?;
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some(format!("users={}", body.users.len())),
                None,
            );
            Ok(body)
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    pub async fn friends(&self, access_token: &str) -> Result<FriendsResponse, MinosError> {
        let path = "/v1/friends/query";
        let url = format!("{}{path}", self.base);
        let trace_id = start_http_trace(Method::POST.as_str(), path, None, None);
        let request = self.request_without_body(Method::POST, &url, Some(access_token))?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let body: FriendsResponse = decode_success_json(resp, "FriendsResponse").await?;
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some(format!("friends={}", body.friends.len())),
                None,
            );
            Ok(body)
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    pub async fn register_agent(
        &self,
        access_token: &str,
        req: RegisterAgentRequest,
    ) -> Result<AgentSummary, MinosError> {
        let path = "/v1/agents";
        let url = format!("{}{path}", self.base);
        let trace_id = start_http_trace(Method::POST.as_str(), path, None, Some(req.name.clone()));
        let request = self.request_with_json(Method::POST, &url, Some(access_token), &req)?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let body: AgentSummary = decode_success_json(resp, "AgentSummary").await?;
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some(body.agent_id.clone()),
                None,
            );
            Ok(body)
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    pub async fn list_agents(&self, access_token: &str) -> Result<ListAgentsResponse, MinosError> {
        let path = "/v1/agents/query";
        let url = format!("{}{path}", self.base);
        let trace_id = start_http_trace(Method::POST.as_str(), path, None, None);
        let request = self.request_without_body(Method::POST, &url, Some(access_token))?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let body: ListAgentsResponse = decode_success_json(resp, "ListAgentsResponse").await?;
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some(format!("agents={}", body.agents.len())),
                None,
            );
            Ok(body)
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    pub async fn create_friend_request(
        &self,
        access_token: &str,
        req: CreateFriendRequestRequest,
    ) -> Result<minos_protocol::FriendRequestSummary, MinosError> {
        let path = "/v1/friend-requests";
        let url = format!("{}{path}", self.base);
        let trace_id = start_http_trace(
            Method::POST.as_str(),
            path,
            None,
            Some(req.target_minos_id.clone()),
        );
        let request = self.request_with_json(Method::POST, &url, Some(access_token), &req)?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let body = decode_success_json(resp, "FriendRequestSummary").await?;
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some("request sent".into()),
                None,
            );
            Ok(body)
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    pub async fn friend_requests(
        &self,
        access_token: &str,
    ) -> Result<FriendRequestsResponse, MinosError> {
        let path = "/v1/friend-requests/query";
        let url = format!("{}{path}", self.base);
        let trace_id = start_http_trace(Method::POST.as_str(), path, None, None);
        let request = self.request_without_body(Method::POST, &url, Some(access_token))?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let body: FriendRequestsResponse =
                decode_success_json(resp, "FriendRequestsResponse").await?;
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some(format!(
                    "incoming={} outgoing={}",
                    body.incoming.len(),
                    body.outgoing.len()
                )),
                None,
            );
            Ok(body)
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    pub async fn accept_friend_request(
        &self,
        access_token: &str,
        request_id: &str,
    ) -> Result<minos_protocol::FriendRequestSummary, MinosError> {
        self.resolve_friend_request(access_token, request_id, true)
            .await
    }

    pub async fn reject_friend_request(
        &self,
        access_token: &str,
        request_id: &str,
    ) -> Result<minos_protocol::FriendRequestSummary, MinosError> {
        self.resolve_friend_request(access_token, request_id, false)
            .await
    }

    async fn resolve_friend_request(
        &self,
        access_token: &str,
        request_id: &str,
        accept: bool,
    ) -> Result<minos_protocol::FriendRequestSummary, MinosError> {
        let action = if accept { "accept" } else { "reject" };
        let path = format!("/v1/friend-requests/{request_id}/{action}");
        let url = format!("{}{}", self.base, path);
        let trace_id = start_http_trace(Method::POST.as_str(), &path, None, None);
        let request = self.request_with_json(
            Method::POST,
            &url,
            Some(access_token),
            &serde_json::json!({}),
        )?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let body = decode_success_json(resp, "FriendRequestSummary").await?;
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some(action.into()),
                None,
            );
            Ok(body)
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    pub async fn conversations(
        &self,
        access_token: &str,
    ) -> Result<ConversationsResponse, MinosError> {
        let path = "/v1/conversations/query";
        let url = format!("{}{path}", self.base);
        let trace_id = start_http_trace(Method::POST.as_str(), path, None, None);
        let request = self.request_without_body(Method::POST, &url, Some(access_token))?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let body: ConversationsResponse =
                decode_success_json(resp, "ConversationsResponse").await?;
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some(format!("conversations={}", body.conversations.len())),
                None,
            );
            Ok(body)
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    pub async fn ensure_direct_conversation(
        &self,
        access_token: &str,
        req: EnsureDirectConversationRequest,
    ) -> Result<ConversationResponse, MinosError> {
        let path = "/v1/conversations/direct";
        let url = format!("{}{path}", self.base);
        let trace_id = start_http_trace(
            Method::POST.as_str(),
            path,
            None,
            Some(req.friend_account_id.clone()),
        );
        let request = self.request_with_json(Method::POST, &url, Some(access_token), &req)?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let body: ConversationResponse =
                decode_success_json(resp, "ConversationResponse").await?;
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some(body.conversation_id.clone()),
                None,
            );
            Ok(body)
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    pub async fn create_group_conversation(
        &self,
        access_token: &str,
        req: CreateGroupConversationRequest,
    ) -> Result<ConversationResponse, MinosError> {
        let path = "/v1/conversations/group";
        let url = format!("{}{path}", self.base);
        let trace_id = start_http_trace(Method::POST.as_str(), path, None, Some(req.title.clone()));
        let request = self.request_with_json(Method::POST, &url, Some(access_token), &req)?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let body: ConversationResponse =
                decode_success_json(resp, "ConversationResponse").await?;
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some(body.conversation_id.clone()),
                None,
            );
            Ok(body)
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    pub async fn add_group_member(
        &self,
        access_token: &str,
        conversation_id: &str,
        req: AddGroupMemberRequest,
    ) -> Result<(), MinosError> {
        let path = format!("/v1/conversations/{conversation_id}/members/add");
        let url = format!("{}{}", self.base, path);
        let trace_id = start_http_trace(
            Method::POST.as_str(),
            &path,
            Some(conversation_id.into()),
            Some(req.member_account_id.clone()),
        );
        let request = self.request_with_json(Method::POST, &url, Some(access_token), &req)?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some("member added".into()),
                Some(conversation_id.into()),
            );
            Ok(())
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    pub async fn conversation_members(
        &self,
        access_token: &str,
        conversation_id: &str,
    ) -> Result<ConversationMembersResponse, MinosError> {
        let path = format!("/v1/conversations/{conversation_id}/members/query");
        let url = format!("{}{}", self.base, path);
        let trace_id = start_http_trace(
            Method::POST.as_str(),
            &path,
            Some(conversation_id.into()),
            None,
        );
        let request = self.request_without_body(Method::POST, &url, Some(access_token))?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let body: ConversationMembersResponse =
                decode_success_json(resp, "ConversationMembersResponse").await?;
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some(format!("members={}", body.members.len())),
                Some(conversation_id.into()),
            );
            Ok(body)
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    pub async fn list_conversation_agents(
        &self,
        access_token: &str,
        conversation_id: &str,
    ) -> Result<ConversationAgentMembersResponse, MinosError> {
        let path = format!("/v1/conversations/{conversation_id}/agents");
        let url = format!("{}{}", self.base, path);
        let trace_id = start_http_trace(
            Method::POST.as_str(),
            &path,
            Some(conversation_id.into()),
            None,
        );
        let request = self.request_without_body(Method::POST, &url, Some(access_token))?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let body: ConversationAgentMembersResponse =
                decode_success_json(resp, "ConversationAgentMembersResponse").await?;
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some(format!("agents={}", body.agents.len())),
                Some(conversation_id.into()),
            );
            Ok(body)
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    pub async fn add_agent_to_group(
        &self,
        access_token: &str,
        conversation_id: &str,
        req: AddAgentToGroupRequest,
    ) -> Result<(), MinosError> {
        let path = format!("/v1/conversations/{conversation_id}/agents/add");
        let url = format!("{}{}", self.base, path);
        let trace_id = start_http_trace(
            Method::POST.as_str(),
            &path,
            Some(conversation_id.into()),
            Some(req.agent_id.clone()),
        );
        let request = self.request_with_json(Method::POST, &url, Some(access_token), &req)?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some("agent added".into()),
                Some(conversation_id.into()),
            );
            Ok(())
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    pub async fn remove_agent_from_group(
        &self,
        access_token: &str,
        conversation_id: &str,
        req: RemoveAgentFromGroupRequest,
    ) -> Result<(), MinosError> {
        let path = format!("/v1/conversations/{conversation_id}/agents/remove");
        let url = format!("{}{}", self.base, path);
        let trace_id = start_http_trace(
            Method::POST.as_str(),
            &path,
            Some(conversation_id.into()),
            Some(req.agent_id.clone()),
        );
        let request = self.request_with_json(Method::POST, &url, Some(access_token), &req)?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some("agent removed".into()),
                Some(conversation_id.into()),
            );
            Ok(())
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    pub async fn mark_conversation_read(
        &self,
        access_token: &str,
        conversation_id: &str,
    ) -> Result<ConversationReadResponse, MinosError> {
        let path = format!("/v1/conversations/{conversation_id}/read");
        let url = format!("{}{}", self.base, path);
        let trace_id = start_http_trace(
            Method::POST.as_str(),
            &path,
            Some(conversation_id.into()),
            Some("mark_read".into()),
        );
        let request = self.request_without_body(Method::POST, &url, Some(access_token))?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let body: ConversationReadResponse =
                decode_success_json(resp, "ConversationReadResponse").await?;
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                body.last_read_at_ms
                    .map(|ts| format!("last_read_at_ms={ts}")),
                Some(conversation_id.into()),
            );
            Ok(body)
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    pub async fn list_chat_messages(
        &self,
        access_token: &str,
        conversation_id: &str,
        before_ts_ms: Option<i64>,
        limit: u32,
    ) -> Result<ListChatMessagesResponse, MinosError> {
        let path = format!("/v1/conversations/{conversation_id}/messages/query");
        let url = format!("{}{}", self.base, path);
        let trace_id = start_http_trace(
            Method::POST.as_str(),
            &path,
            Some(conversation_id.into()),
            None,
        );
        let request = self.request_with_json(
            Method::POST,
            &url,
            Some(access_token),
            &ListChatMessagesRequest {
                before_ts_ms,
                limit: Some(limit),
            },
        )?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let body: ListChatMessagesResponse =
                decode_success_json(resp, "ListChatMessagesResponse").await?;
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some(format!("messages={}", body.messages.len())),
                Some(conversation_id.into()),
            );
            Ok(body)
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    pub async fn send_chat_message(
        &self,
        access_token: &str,
        conversation_id: &str,
        req: SendChatMessageRequest,
    ) -> Result<minos_protocol::ChatMessageSummary, MinosError> {
        let path = format!("/v1/conversations/{conversation_id}/messages");
        let url = format!("{}{}", self.base, path);
        let trace_id = start_http_trace(
            Method::POST.as_str(),
            &path,
            Some(conversation_id.into()),
            Some(format!("len={}", req.text.len())),
        );
        let request = self.request_with_json(Method::POST, &url, Some(access_token), &req)?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let body = decode_success_json(resp, "ChatMessageSummary").await?;
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some("message sent".into()),
                Some(conversation_id.into()),
            );
            Ok(body)
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    pub async fn recall_chat_message(
        &self,
        access_token: &str,
        conversation_id: &str,
        message_id: &str,
    ) -> Result<minos_protocol::ChatMessageSummary, MinosError> {
        let path = format!("/v1/conversations/{conversation_id}/messages/{message_id}/recall");
        let url = format!("{}{}", self.base, path);
        let trace_id = start_http_trace(
            Method::POST.as_str(),
            &path,
            Some(conversation_id.into()),
            Some(format!("recall={message_id}")),
        );
        let request = self.request_without_body(Method::POST, &url, Some(access_token))?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let body = decode_success_json(resp, "ChatMessageSummary").await?;
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some("message recalled".into()),
                Some(conversation_id.into()),
            );
            Ok(body)
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    /// `POST /v1/realtime/ws-ticket` — obtain a short-lived ticket for the
    /// `/ws/client` WebSocket upgrade. The ticket replaces header-based auth.
    pub async fn fetch_ws_ticket(
        &self,
        access_token: &str,
        installation_id: &str,
    ) -> Result<RealtimeWsTicketResponse, MinosError> {
        let path = "/v1/realtime/ws-ticket";
        let url = format!("{}{path}", self.base);
        let trace_id =
            start_http_trace(Method::POST.as_str(), path, None, Some("ws-ticket".into()));
        let body = RealtimeWsTicketRequest {
            installation_id: Some(installation_id.to_string()),
        };
        let request = self.request_with_json(Method::POST, &url, Some(access_token), &body)?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let envelope: WsTicketEnvelope = decode_success_json(resp, "WsTicketEnvelope").await?;
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some("ws-ticket obtained".into()),
                None,
            );
            Ok(RealtimeWsTicketResponse {
                ticket: envelope.data.ticket,
                gateway_url: Some(envelope.data.gateway_url),
            })
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    pub async fn list_clis_http(
        &self,
        access_token: &str,
        request_body: ListHostClisRequest,
    ) -> Result<minos_protocol::ListClisResponse, MinosError> {
        let path = "/v1/host-commands/list-clis";
        let url = format!("{}{path}", self.base);
        let trace_id =
            start_http_trace(Method::POST.as_str(), path, None, Some("list clis".into()));
        let request =
            self.request_with_json(Method::POST, &url, Some(access_token), &request_body)?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let body = decode_success_json(resp, "ListClisResponse").await?;
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some("host cli list".into()),
                None,
            );
            Ok(body)
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    pub async fn list_host_skills_http(
        &self,
        access_token: &str,
        request_body: ListHostSkillsCommandRequest,
    ) -> Result<ListHostSkillsResponse, MinosError> {
        let path = "/v1/host-commands/list-host-skills";
        let url = format!("{}{path}", self.base);
        let trace_id = start_http_trace(
            Method::POST.as_str(),
            path,
            None,
            Some(format!("force_reload={}", request_body.force_reload)),
        );
        let request =
            self.request_with_json(Method::POST, &url, Some(access_token), &request_body)?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let body = decode_success_json(resp, "ListHostSkillsResponse").await?;
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some("host skills listed".into()),
                None,
            );
            Ok(body)
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    pub async fn write_host_skill_config_http(
        &self,
        access_token: &str,
        request_body: WriteHostSkillConfigCommandRequest,
    ) -> Result<WriteHostSkillConfigResponse, MinosError> {
        let path = "/v1/host-commands/write-host-skill-config";
        let url = format!("{}{path}", self.base);
        let trace_id = start_http_trace(
            Method::POST.as_str(),
            path,
            None,
            Some(format!("enabled={}", request_body.enabled)),
        );
        let request =
            self.request_with_json(Method::POST, &url, Some(access_token), &request_body)?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let body = decode_success_json(resp, "WriteHostSkillConfigResponse").await?;
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some("host skill config updated".into()),
                None,
            );
            Ok(body)
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    // ─────────────────────────── agent session endpoints ──────────────────

    /// `POST /v1/agent-sessions/send-input` — send a user message into an
    /// active agent session via the backend's durable command queue.
    pub async fn send_agent_input(
        &self,
        access_token: &str,
        session_id: &str,
        text: &str,
    ) -> Result<SendAgentInputResponse, MinosError> {
        let path = "/v1/agent-sessions/send-input";
        let url = format!("{}{path}", self.base);
        let trace_id = start_http_trace(
            Method::POST.as_str(),
            path,
            None,
            Some(format!("session={session_id}")),
        );
        let body = SendAgentInputRequest {
            session_id: session_id.into(),
            text: text.into(),
            client_request_id: Uuid::new_v4().to_string(),
        };
        let request = self.request_with_json(Method::POST, &url, Some(access_token), &body)?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            let body = decode_success_json(resp, "SendAgentInputResponse").await?;
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some("agent input sent".into()),
                None,
            );
            Ok(body)
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    /// `POST /v1/agent-sessions/stop` — stop (interrupt or close) an agent
    /// session via the backend's durable command queue.
    pub async fn stop_agent_session(
        &self,
        access_token: &str,
        session_id: &str,
    ) -> Result<(), MinosError> {
        let path = "/v1/agent-sessions/stop";
        let url = format!("{}{path}", self.base);
        let trace_id = start_http_trace(
            Method::POST.as_str(),
            path,
            None,
            Some(format!("session={session_id}")),
        );
        let body = StopAgentSessionRequest {
            session_id: session_id.into(),
        };
        let request = self.request_with_json(Method::POST, &url, Some(access_token), &body)?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status.is_success() {
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some("agent session stopped".into()),
                None,
            );
            Ok(())
        } else {
            let error = decode_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    // ─────────────────────────── auth endpoints ───────────────────────────

    /// `POST /v1/auth/register` — create an account on the backend.
    /// Bearer-only post ADR-0020; the iOS rail no longer carries
    /// `X-Device-Secret`. Spec §5.2.
    pub async fn register(&self, email: &str, password: &str) -> Result<AuthResponse, MinosError> {
        let url = format!("{}/v1/auth/register", self.base);
        let trace_id = start_http_trace(
            Method::POST.as_str(),
            "/v1/auth/register",
            None,
            Some(format!("email={email}")),
        );
        let body = AuthRequest {
            email: email.into(),
            password: password.into(),
        };
        let request = self.request_with_json(Method::POST, &url, None, &body)?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        decode_auth_response(resp, trace_id).await
    }

    /// `POST /v1/auth/login` — authenticate an existing account.
    /// Bearer-only post ADR-0020. Spec §5.2.
    pub async fn login(&self, email: &str, password: &str) -> Result<AuthResponse, MinosError> {
        let url = format!("{}/v1/auth/login", self.base);
        let trace_id = start_http_trace(
            Method::POST.as_str(),
            "/v1/auth/login",
            None,
            Some(format!("email={email}")),
        );
        let body = AuthRequest {
            email: email.into(),
            password: password.into(),
        };
        let request = self.request_with_json(Method::POST, &url, None, &body)?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        decode_auth_response(resp, trace_id).await
    }

    /// `POST /v1/auth/refresh` — rotate the bearer + refresh pair.
    /// Bearer-only post ADR-0020.
    pub async fn refresh(&self, refresh_token: &str) -> Result<RefreshResponse, MinosError> {
        let url = format!("{}/v1/auth/refresh", self.base);
        let trace_id = start_http_trace(
            Method::POST.as_str(),
            "/v1/auth/refresh",
            None,
            Some("refresh session".into()),
        );
        let body = RefreshRequest {
            refresh_token: refresh_token.into(),
        };
        let request = self.request_with_json(Method::POST, &url, None, &body)?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        decode_refresh_response(resp, trace_id).await
    }

    /// `POST /v1/auth/logout` — revoke the named refresh token.
    /// Bearer-only post ADR-0020.
    pub async fn logout(&self, access_token: &str, refresh_token: &str) -> Result<(), MinosError> {
        let url = format!("{}/v1/auth/logout", self.base);
        let trace_id = start_http_trace(
            Method::POST.as_str(),
            "/v1/auth/logout",
            None,
            Some("logout current session".into()),
        );
        let body = LogoutRequest {
            refresh_token: refresh_token.into(),
        };
        let request = self.request_with_json(Method::POST, &url, Some(access_token), &body)?;
        let resp = self.execute_with_trace(trace_id, &url, request).await?;
        let status = resp.status();
        if status == StatusCode::NO_CONTENT || status.is_success() {
            request_trace::finish_success(
                trace_id,
                Some(status.as_u16()),
                Some("logged out".into()),
                None,
            );
            Ok(())
        } else {
            let error = decode_kind_error(resp).await;
            request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
            Err(error)
        }
    }

    /// Build a request stamped with the device-id + bearer token. Use this
    /// for any account-aware route the daemon adds in future phases.
    pub fn build_authed_request(
        &self,
        method: Method,
        path: &str,
        access: &str,
    ) -> Result<Request<RequestBody>, MinosError> {
        let url = format!("{}{}", self.base, path);
        self.request_without_body(method, &url, Some(access))
    }

    fn request_with_json<T>(
        &self,
        method: Method,
        url: &str,
        access_token: Option<&str>,
        body: &T,
    ) -> Result<Request<RequestBody>, MinosError>
    where
        T: Serialize,
    {
        let payload = RequestBody::from_json(body).map_err(|e| MinosError::BackendInternal {
            message: format!("encode request body {url}: {e}"),
        })?;
        Self::finish_request(
            self.request_builder(method, url, access_token)
                .header(CONTENT_TYPE, "application/json"),
            payload,
            url,
        )
    }

    fn request_without_body(
        &self,
        method: Method,
        url: &str,
        access_token: Option<&str>,
    ) -> Result<Request<RequestBody>, MinosError> {
        Self::finish_request(
            self.request_builder(method, url, access_token),
            RequestBody::absent(),
            url,
        )
    }

    fn request_builder(
        &self,
        method: Method,
        url: &str,
        access_token: Option<&str>,
    ) -> http::request::Builder {
        let mut req = Request::builder()
            .method(method)
            .uri(url)
            .header("x-device-id", self.device_id.to_string())
            .header("x-device-name", &self.device_name)
            .header("x-device-role", self.device_role);
        if let Some(access_token) = access_token {
            req = req.header("authorization", format!("Bearer {access_token}"));
        }
        req
    }

    fn finish_request(
        req: http::request::Builder,
        body: RequestBody,
        url: &str,
    ) -> Result<Request<RequestBody>, MinosError> {
        req.body(body).map_err(|e| MinosError::BackendInternal {
            message: format!("build request {url}: {e}"),
        })
    }

    async fn execute(
        &self,
        url: &str,
        request: Request<RequestBody>,
    ) -> Result<Response<ResponseBody>, MinosError> {
        self.client
            .execute(request)
            .await
            .map_err(|e| connect_err(url, &e))
    }

    async fn execute_with_trace(
        &self,
        trace_id: u64,
        url: &str,
        request: Request<RequestBody>,
    ) -> Result<Response<ResponseBody>, MinosError> {
        match self.execute(url, request).await {
            Ok(resp) => Ok(resp),
            Err(error) => {
                request_trace::finish_failure(trace_id, None, error.to_string());
                Err(error)
            }
        }
    }
}

fn formal_hosts_to_me_hosts(data: ListHostsData) -> Result<MeHostsResponse, MinosError> {
    let mut hosts = Vec::with_capacity(data.hosts.len());
    for host in data.hosts {
        hosts.push(HostSummary {
            host_device_id: parse_device_id_field(
                "host_installation_id",
                &host.host_installation_id,
            )?,
            host_display_name: host.host_display_name,
            paired_at_ms: host.paired_at_ms,
            paired_via_device_id: parse_device_id_field(
                "linked_via_installation_id",
                &host.linked_via_installation_id,
            )?,
        });
    }
    Ok(MeHostsResponse { hosts })
}

fn append_turn_ui_events(
    ui_events: &mut Vec<UiEventMessage>,
    turn: &AgentTurnMetadata,
    events: Vec<AgentTurnEvent>,
) {
    if events.is_empty() {
        append_turn_summary_ui_events(ui_events, turn);
        return;
    }

    let role = message_role_from_turn_role(&turn.role);
    let mut started_message_ids = HashSet::<String>::new();
    for event in events {
        let message_id = message_id_for_turn_event(turn, &event);
        if started_message_ids.insert(message_id.clone()) {
            ui_events.push(UiEventMessage::MessageStarted {
                message_id: message_id.clone(),
                role,
                started_at_ms: turn.started_at_ms,
            });
        }
        ui_events.extend(ui_events_for_turn_event(&message_id, event));
    }

    if turn_is_complete(turn) {
        let finished_at_ms = turn.finished_at_ms.unwrap_or(turn.started_at_ms);
        for message_id in started_message_ids {
            ui_events.push(UiEventMessage::MessageCompleted {
                message_id,
                finished_at_ms,
            });
        }
    }
}

fn append_turn_summary_ui_events(ui_events: &mut Vec<UiEventMessage>, turn: &AgentTurnMetadata) {
    let Some(text) = turn.summary_text.as_deref().filter(|text| !text.is_empty()) else {
        return;
    };
    ui_events.push(UiEventMessage::MessageStarted {
        message_id: turn.turn_id.clone(),
        role: message_role_from_turn_role(&turn.role),
        started_at_ms: turn.started_at_ms,
    });
    ui_events.push(UiEventMessage::TextDelta {
        message_id: turn.turn_id.clone(),
        text: text.to_string(),
    });
    if turn_is_complete(turn) {
        ui_events.push(UiEventMessage::MessageCompleted {
            message_id: turn.turn_id.clone(),
            finished_at_ms: turn.finished_at_ms.unwrap_or(turn.started_at_ms),
        });
    }
}

fn ui_events_for_turn_event(message_id: &str, event: AgentTurnEvent) -> Vec<UiEventMessage> {
    match event.kind.as_str() {
        "agent_text_delta" => event_text(&event.payload)
            .map(|text| {
                vec![UiEventMessage::TextDelta {
                    message_id: message_id.to_string(),
                    text,
                }]
            })
            .unwrap_or_default(),
        "agent_text_replace" => event_text(&event.payload)
            .map(|text| {
                vec![UiEventMessage::TextReplace {
                    message_id: message_id.to_string(),
                    text,
                }]
            })
            .unwrap_or_default(),
        "agent_reasoning_delta" => event_text(&event.payload)
            .map(|text| {
                vec![UiEventMessage::ReasoningDelta {
                    message_id: message_id.to_string(),
                    text,
                }]
            })
            .unwrap_or_default(),
        "agent_reasoning_replace" => event_text(&event.payload)
            .map(|text| {
                vec![UiEventMessage::ReasoningReplace {
                    message_id: message_id.to_string(),
                    text,
                }]
            })
            .unwrap_or_default(),
        "agent_tool_call" => {
            let tool_call_id = event
                .payload
                .get("tool_call_id")
                .or_else(|| event.payload.get("id"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("tool_call")
                .to_string();
            let name = event
                .payload
                .get("name")
                .or_else(|| event.payload.get("tool_name"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("tool")
                .to_string();
            let args_json = event
                .payload
                .get("args_json")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
                .or_else(|| event.payload.get("args").map(serde_json::Value::to_string))
                .unwrap_or_else(|| "{}".into());
            vec![UiEventMessage::ToolCallPlaced {
                message_id: message_id.to_string(),
                tool_call_id,
                name,
                args_json,
            }]
        }
        "agent_tool_result" | "agent_tool_completed" => {
            let tool_call_id = event
                .payload
                .get("tool_call_id")
                .or_else(|| event.payload.get("id"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("tool_call")
                .to_string();
            let output = event
                .payload
                .get("output")
                .or_else(|| event.payload.get("result"))
                .map(|value| {
                    value
                        .as_str()
                        .map(str::to_string)
                        .unwrap_or_else(|| value.to_string())
                })
                .unwrap_or_default();
            let is_error = event
                .payload
                .get("is_error")
                .or_else(|| event.payload.get("error"))
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);
            vec![UiEventMessage::ToolCallCompleted {
                tool_call_id,
                output,
                is_error,
            }]
        }
        "agent_error" => {
            let code = event
                .payload
                .get("code")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("agent_error")
                .to_string();
            let message = event
                .payload
                .get("message")
                .or_else(|| event.payload.get("detail"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or("agent error")
                .to_string();
            vec![UiEventMessage::Error {
                code,
                message,
                message_id: Some(message_id.to_string()),
            }]
        }
        _ => vec![UiEventMessage::Raw {
            kind: event.kind,
            payload_json: event.payload.to_string(),
        }],
    }
}

fn message_id_for_turn_event(turn: &AgentTurnMetadata, event: &AgentTurnEvent) -> String {
    let candidate = event
        .payload
        .get("message_id")
        .or_else(|| event.payload.get("msg_id"))
        .and_then(serde_json::Value::as_str)
        .unwrap_or(&event.turn_id);
    if candidate.is_empty() {
        turn.turn_id.clone()
    } else {
        candidate.to_string()
    }
}

fn event_text(payload: &serde_json::Value) -> Option<String> {
    payload
        .get("text")
        .or_else(|| payload.get("delta"))
        .or_else(|| payload.get("content"))
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
}

fn message_role_from_turn_role(role: &str) -> MessageRole {
    match role {
        "user" => MessageRole::User,
        "system" => MessageRole::System,
        _ => MessageRole::Assistant,
    }
}

fn turn_is_complete(turn: &AgentTurnMetadata) -> bool {
    matches!(
        turn.status.as_str(),
        "completed" | "failed" | "cancelled" | "canceled"
    ) || turn.finished_at_ms.is_some()
}

fn optional_u64_to_i64(value: Option<u64>, field: &str) -> Result<Option<i64>, MinosError> {
    value
        .map(|value| {
            i64::try_from(value).map_err(|_| MinosError::BackendInternal {
                message: format!("{field} exceeds i64 range: {value}"),
            })
        })
        .transpose()
}

fn map_agent_session_not_found(error: MinosError, thread_id: &str) -> MinosError {
    match error {
        MinosError::RpcCallFailed { method, message }
            if method.contains("404") && message.contains("agent_session_not_found") =>
        {
            MinosError::ThreadNotFound {
                thread_id: thread_id.to_string(),
            }
        }
        other => other,
    }
}

fn parse_device_id_field(field: &str, value: &str) -> Result<DeviceId, MinosError> {
    Uuid::parse_str(value)
        .map(DeviceId)
        .map_err(|error| MinosError::BackendInternal {
            message: format!("decode ListHostsData.{field}: {error}"),
        })
}

fn connect_err(url: &str, e: &WireError) -> MinosError {
    match e.response_status() {
        Some(StatusCode::UNAUTHORIZED | StatusCode::FOUND | StatusCode::FORBIDDEN) => {
            MinosError::Unauthorized {
                reason: format!("{url}: {e}"),
            }
        }
        _ => MinosError::ConnectFailed {
            url: url.into(),
            message: e.to_string(),
        },
    }
}

async fn decode_success_json<T>(
    resp: Response<ResponseBody>,
    type_name: &str,
) -> Result<T, MinosError>
where
    T: DeserializeOwned,
{
    resp.into_body()
        .json::<T>()
        .await
        .map_err(|e| MinosError::BackendInternal {
            message: format!("decode {type_name}: {e}"),
        })
}

async fn decode_error(resp: Response<ResponseBody>) -> MinosError {
    let status = resp.status();
    let body: Result<ErrorEnvelope, _> = resp.into_body().json().await;
    match body {
        Ok(env) => {
            let detail = format!("{}: {}", env.error.code, env.error.message);
            if status == StatusCode::UNAUTHORIZED {
                MinosError::Unauthorized { reason: detail }
            } else {
                MinosError::RpcCallFailed {
                    method: format!("http {status}"),
                    message: detail,
                }
            }
        }
        Err(_) => MinosError::BackendInternal {
            message: format!("backend {status}"),
        },
    }
}

/// Decode an `AuthResponse` from the backend, mapping `kind` strings on
/// the failure path to typed `MinosError` variants. Spec §5.4, §8.1.
async fn decode_auth_response(
    resp: Response<ResponseBody>,
    trace_id: u64,
) -> Result<AuthResponse, MinosError> {
    let status = resp.status();
    if status.is_success() {
        let auth = decode_success_json::<AuthResponse>(resp, "AuthResponse").await?;
        request_trace::finish_success(
            trace_id,
            Some(status.as_u16()),
            Some(format!(
                "account={} expires_in={}s",
                auth.account.email, auth.expires_in
            )),
            None,
        );
        return Ok(auth);
    }
    let error = decode_kind_error(resp).await;
    request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
    Err(error)
}

/// Decode a `RefreshResponse` from the backend, mapping `kind` strings on
/// the failure path to typed `MinosError` variants.
async fn decode_refresh_response(
    resp: Response<ResponseBody>,
    trace_id: u64,
) -> Result<RefreshResponse, MinosError> {
    let status = resp.status();
    if status.is_success() {
        let refresh = decode_success_json::<RefreshResponse>(resp, "RefreshResponse").await?;
        request_trace::finish_success(
            trace_id,
            Some(status.as_u16()),
            Some(format!("expires_in={}s", refresh.expires_in)),
            None,
        );
        return Ok(refresh);
    }
    let error = decode_kind_error(resp).await;
    request_trace::finish_failure(trace_id, Some(status.as_u16()), error.to_string());
    Err(error)
}

/// Map an HTTP error response that carries either the old `{ "kind": "..." }`
/// body or the current `{ "error": { "code": "..." } }` envelope to a typed
/// `MinosError`. Used by every `/v1/auth/*` endpoint. Spec §8.1.
async fn decode_kind_error(resp: Response<ResponseBody>) -> MinosError {
    let (parts, body) = resp.into_parts();
    let retry_after = parts
        .headers
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(60);
    let body: serde_json::Value = body.json().await.unwrap_or(serde_json::Value::Null);
    let kind = body
        .get("kind")
        .and_then(|v| v.as_str())
        .or_else(|| {
            body.get("error")
                .and_then(|v| v.get("code"))
                .and_then(|v| v.as_str())
        })
        .unwrap_or("unknown")
        .to_string();
    match (parts.status.as_u16(), kind.as_str()) {
        (400, "weak_password") => MinosError::WeakPassword,
        (401, "invalid_credentials") => MinosError::InvalidCredentials,
        (401, "invalid_refresh") => MinosError::AuthRefreshFailed {
            message: "invalid refresh token".into(),
        },
        (401, _) => MinosError::Unauthorized {
            reason: format!("auth failed ({kind})"),
        },
        (409, "email_taken") => MinosError::EmailTaken,
        (429, _) => MinosError::RateLimited {
            retry_after_s: retry_after,
        },
        _ => MinosError::BackendInternal {
            message: format!("{} {kind}", parts.status),
        },
    }
}

pub(crate) fn http_base(ws_url: &str) -> Option<String> {
    let url = url::Url::parse(ws_url).ok()?;
    let scheme = match url.scheme() {
        "ws" => "http",
        "wss" => "https",
        other => other,
    };
    let host = url.host_str()?;
    let port = url.port().map(|p| format!(":{p}")).unwrap_or_default();
    Some(format!("{scheme}://{host}{port}"))
}

fn agent_name_from_session_agent_id(agent_id: &str) -> Option<AgentName> {
    match agent_id {
        "agent_codex" | "codex" => Some(AgentName::Codex),
        "agent_claude" | "claude" => Some(AgentName::Claude),
        "agent_gemini" | "gemini" => Some(AgentName::Gemini),
        "agent_opencode" | "opencode" => Some(AgentName::Opencode),
        _ => None,
    }
}

fn start_http_trace(
    method: &str,
    target: &str,
    thread_id: Option<String>,
    request_summary: Option<String>,
) -> u64 {
    request_trace::start(
        RequestTransport::Http,
        method,
        target,
        thread_id,
        request_summary,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_agent_session_summary_maps_to_thread_summary() {
        let summary = FormalAgentSessionSummary {
            session_id: "sess-1".into(),
            conversation_id: "conv-1".into(),
            project_id: Some("proj-1".into()),
            agent_id: Some("agent_codex".into()),
            agent: Some(AgentName::Codex),
            status: "running".into(),
            started_at_ms: 100,
            ended_at_ms: None,
            title: Some("Thread title".into()),
            last_activity_at_ms: 250,
            message_count: 3,
            end_reason: None,
        };

        let thread = summary.into_thread_summary().unwrap();
        assert_eq!(thread.thread_id, "sess-1");
        assert_eq!(thread.agent, AgentName::Codex);
        assert_eq!(thread.title.as_deref(), Some("Thread title"));
        assert_eq!(thread.first_ts_ms, 100);
        assert_eq!(thread.last_ts_ms, 250);
        assert_eq!(thread.message_count, 3);
        assert_eq!(thread.ended_at_ms, None);
    }

    #[test]
    fn project_agent_session_summary_falls_back_to_agent_id_slug() {
        let summary = FormalAgentSessionSummary {
            session_id: "sess-2".into(),
            conversation_id: "conv-2".into(),
            project_id: Some("proj-2".into()),
            agent_id: Some("agent_claude".into()),
            agent: None,
            status: "ended".into(),
            started_at_ms: 10,
            ended_at_ms: Some(20),
            title: None,
            last_activity_at_ms: 20,
            message_count: 0,
            end_reason: None,
        };

        let thread = summary.into_thread_summary().unwrap();
        assert_eq!(thread.agent, AgentName::Claude);
    }

    #[test]
    fn turn_summary_maps_to_user_message_events() {
        let turn = AgentTurnMetadata {
            turn_id: "turn-user-1".into(),
            turn_seq: 1,
            role: "user".into(),
            status: "completed".into(),
            started_at_ms: 100,
            finished_at_ms: Some(110),
            summary_text: Some("hello".into()),
        };
        let mut ui_events = Vec::new();

        append_turn_ui_events(&mut ui_events, &turn, Vec::new());

        assert_eq!(ui_events.len(), 3);
        assert!(matches!(
            &ui_events[0],
            UiEventMessage::MessageStarted {
                message_id,
                role: MessageRole::User,
                started_at_ms: 100
            } if message_id == "turn-user-1"
        ));
        assert!(matches!(
            &ui_events[1],
            UiEventMessage::TextDelta { message_id, text }
                if message_id == "turn-user-1" && text == "hello"
        ));
        assert!(matches!(
            &ui_events[2],
            UiEventMessage::MessageCompleted {
                message_id,
                finished_at_ms: 110
            } if message_id == "turn-user-1"
        ));
    }

    #[test]
    fn formal_turn_events_map_to_renderable_assistant_events() {
        let turn = AgentTurnMetadata {
            turn_id: "turn-assistant-1".into(),
            turn_seq: 2,
            role: "assistant".into(),
            status: "completed".into(),
            started_at_ms: 200,
            finished_at_ms: Some(260),
            summary_text: None,
        };
        let events = vec![AgentTurnEvent {
            turn_id: "turn-assistant-1".into(),
            kind: "agent_text_delta".into(),
            payload: serde_json::json!({
                "turn_id": "turn-assistant-1",
                "seq": 1,
                "message_id": "msg-assistant-1",
                "delta": "hi"
            }),
            created_at_ms: 210,
        }];
        let mut ui_events = Vec::new();

        append_turn_ui_events(&mut ui_events, &turn, events);

        assert_eq!(ui_events.len(), 3);
        assert!(matches!(
            &ui_events[0],
            UiEventMessage::MessageStarted {
                message_id,
                role: MessageRole::Assistant,
                started_at_ms: 200
            } if message_id == "msg-assistant-1"
        ));
        assert!(matches!(
            &ui_events[1],
            UiEventMessage::TextDelta { message_id, text }
                if message_id == "msg-assistant-1" && text == "hi"
        ));
        assert!(matches!(
            &ui_events[2],
            UiEventMessage::MessageCompleted {
                message_id,
                finished_at_ms: 260
            } if message_id == "msg-assistant-1"
        ));
    }

    #[test]
    fn formal_hosts_response_maps_to_existing_host_summary_shape() {
        let host_id = DeviceId::new();
        let mobile_id = DeviceId::new();

        let response = formal_hosts_to_me_hosts(ListHostsData {
            hosts: vec![FormalHostSummary {
                host_installation_id: host_id.to_string(),
                host_display_name: "Mac Studio".into(),
                paired_at_ms: 123,
                linked_via_installation_id: mobile_id.to_string(),
            }],
        })
        .unwrap();

        assert_eq!(response.hosts.len(), 1);
        assert_eq!(response.hosts[0].host_device_id, host_id);
        assert_eq!(response.hosts[0].host_display_name, "Mac Studio");
        assert_eq!(response.hosts[0].paired_at_ms, 123);
        assert_eq!(response.hosts[0].paired_via_device_id, mobile_id);
    }

    #[test]
    fn formal_hosts_response_rejects_bad_device_ids() {
        let err = formal_hosts_to_me_hosts(ListHostsData {
            hosts: vec![FormalHostSummary {
                host_installation_id: "not-a-uuid".into(),
                host_display_name: "Mac Studio".into(),
                paired_at_ms: 123,
                linked_via_installation_id: DeviceId::new().to_string(),
            }],
        })
        .expect_err("invalid host id must not be silently accepted");

        assert!(
            matches!(err, MinosError::BackendInternal { ref message } if message.contains("host_installation_id")),
            "unexpected error: {err:?}"
        );
    }
}
