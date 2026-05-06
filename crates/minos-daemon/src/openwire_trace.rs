use std::sync::Arc;

use http::{Method, Request, Response};
use openwire::websocket::WebSocketHandshake;
use openwire::{
    CallContext, EventListener, EventListenerFactory, LogLevel, LoggerInterceptor, RequestBody,
    ResponseBody, WireError,
};
use openwire_core::websocket::{CloseInitiator, MessageKind, WebSocketError};

#[derive(Debug, Clone, Copy)]
pub(crate) struct OpenwireTraceFactory {
    component: &'static str,
}

impl OpenwireTraceFactory {
    pub(crate) const fn new(component: &'static str) -> Self {
        Self { component }
    }
}

pub(crate) fn logger_interceptor(component: &'static str) -> LoggerInterceptor {
    LoggerInterceptor::with_logger(LogLevel::Body, move |message: &str| {
        tracing::info!(
            target: "minos_daemon::network",
            component,
            message = %message,
            "openwire http"
        );
    })
}

impl EventListenerFactory for OpenwireTraceFactory {
    fn create(&self, request: &Request<RequestBody>) -> Arc<dyn EventListener> {
        Arc::new(OpenwireTraceListener {
            component: self.component,
            method: request.method().clone(),
            uri: request.uri().to_string(),
        })
    }
}

#[derive(Debug)]
struct OpenwireTraceListener {
    component: &'static str,
    method: Method,
    uri: String,
}

impl EventListener for OpenwireTraceListener {
    fn call_start(&self, ctx: &CallContext, _request: &Request<RequestBody>) {
        tracing::debug!(
            target: "minos_daemon::network",
            component = self.component,
            call_id = %ctx.call_id(),
            method = %self.method,
            uri = %self.uri,
            "openwire call start"
        );
    }

    fn call_end(&self, ctx: &CallContext) {
        tracing::debug!(
            target: "minos_daemon::network",
            component = self.component,
            call_id = %ctx.call_id(),
            method = %self.method,
            uri = %self.uri,
            "openwire call complete"
        );
    }

    fn call_failed(&self, ctx: &CallContext, error: &WireError) {
        tracing::warn!(
            target: "minos_daemon::network",
            component = self.component,
            call_id = %ctx.call_id(),
            method = %self.method,
            uri = %self.uri,
            response_status = ?error.response_status().map(|status| status.as_u16()),
            error = %error,
            "openwire call failed"
        );
    }

    fn response_headers_end(&self, ctx: &CallContext, response: &Response<ResponseBody>) {
        tracing::debug!(
            target: "minos_daemon::network",
            component = self.component,
            call_id = %ctx.call_id(),
            method = %self.method,
            uri = %self.uri,
            status = %response.status(),
            "openwire response headers"
        );
    }

    fn websocket_open(&self, ctx: &CallContext, handshake: &WebSocketHandshake) {
        tracing::info!(
            target: "minos_daemon::network",
            component = self.component,
            call_id = %ctx.call_id(),
            method = %self.method,
            uri = %self.uri,
            status = %handshake.status(),
            "openwire websocket open"
        );
    }

    fn websocket_message_sent(&self, ctx: &CallContext, kind: MessageKind, payload_len: usize) {
        tracing::debug!(
            target: "minos_daemon::network",
            component = self.component,
            call_id = %ctx.call_id(),
            method = %self.method,
            uri = %self.uri,
            ?kind,
            payload_len,
            "openwire websocket message sent"
        );
    }

    fn websocket_message_received(&self, ctx: &CallContext, kind: MessageKind, payload_len: usize) {
        tracing::debug!(
            target: "minos_daemon::network",
            component = self.component,
            call_id = %ctx.call_id(),
            method = %self.method,
            uri = %self.uri,
            ?kind,
            payload_len,
            "openwire websocket message received"
        );
    }

    fn websocket_closing(
        &self,
        ctx: &CallContext,
        code: u16,
        reason: &str,
        initiator: CloseInitiator,
    ) {
        tracing::info!(
            target: "minos_daemon::network",
            component = self.component,
            call_id = %ctx.call_id(),
            method = %self.method,
            uri = %self.uri,
            code,
            reason,
            ?initiator,
            "openwire websocket closing"
        );
    }

    fn websocket_closed(&self, ctx: &CallContext, code: u16, reason: &str) {
        tracing::info!(
            target: "minos_daemon::network",
            component = self.component,
            call_id = %ctx.call_id(),
            method = %self.method,
            uri = %self.uri,
            code,
            reason,
            "openwire websocket closed"
        );
    }

    fn websocket_failed(&self, ctx: &CallContext, error: &WebSocketError) {
        tracing::warn!(
            target: "minos_daemon::network",
            component = self.component,
            call_id = %ctx.call_id(),
            method = %self.method,
            uri = %self.uri,
            error = %error,
            "openwire websocket failed"
        );
    }
}
