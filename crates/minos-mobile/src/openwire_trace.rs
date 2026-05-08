use std::error::Error as _;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;

use http::{Method, Request, Response};
use openwire::{
    CallContext, ConnectionId, EventListener, EventListenerFactory, LogLevel,
    LoggerInterceptor, RequestBody, ResponseBody, WireError,
};

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
            target: "minos_mobile::network",
            component,
            message = %message,
            "openwire http"
        );
    })
}

fn error_source_chain(error: &WireError) -> String {
    let mut chain = Vec::new();
    let mut source = error.source();
    while let Some(err) = source {
        chain.push(err.to_string());
        source = err.source();
    }

    if chain.is_empty() {
        "<none>".to_owned()
    } else {
        chain.join(" <- ")
    }
}

fn log_wire_error(
    component: &'static str,
    ctx: &CallContext,
    method: &Method,
    uri: &str,
    event: &'static str,
    error: &WireError,
    addr: Option<SocketAddr>,
    server_name: Option<&str>,
    reason: Option<&str>,
) {
    let source_chain = error_source_chain(error);

    tracing::warn!(
        target: "minos_mobile::network",
        component,
        call_id = %ctx.call_id(),
        method = %method,
        uri,
        addr = ?addr,
        server_name = ?server_name,
        reason = ?reason,
        error_kind = %error.kind(),
        error_phase = %error.phase(),
        error_message = %error.message(),
        error_display = %error,
        establishment_stage = ?error.establishment_stage(),
        establishment_retryable = error.is_retryable_establishment(),
        connect_timeout = error.is_connect_timeout(),
        authority = ?error.authority(),
        proxy_addr = ?error.proxy_addr(),
        response_status = ?error.response_status().map(|status| status.as_u16()),
        request_committed = error.request_committed(),
        source_chain = %source_chain,
        event
    );
}

impl EventListenerFactory for OpenwireTraceFactory {
    fn create(&self, request: &Request<RequestBody>) -> Arc<dyn EventListener> {
        Arc::new(OpenwireTraceListener {
            component: self.component,
            method: request.method().clone(),
            uri: request.uri().to_string(),
            dial_attempt: AtomicU32::new(0),
            retry_count: AtomicU32::new(0),
            redirect_count: AtomicU32::new(0),
        })
    }
}

#[derive(Debug)]
struct OpenwireTraceListener {
    component: &'static str,
    method: Method,
    uri: String,
    dial_attempt: AtomicU32,
    retry_count: AtomicU32,
    redirect_count: AtomicU32,
}

impl OpenwireTraceListener {
    fn next_dial_attempt(&self) -> u32 {
        self.dial_attempt.fetch_add(1, Ordering::Relaxed) + 1
    }

    fn current_dial_attempt(&self) -> u32 {
        self.dial_attempt.load(Ordering::Relaxed)
    }

    fn retry_count(&self) -> u32 {
        self.retry_count.load(Ordering::Relaxed)
    }

    fn redirect_count(&self) -> u32 {
        self.redirect_count.load(Ordering::Relaxed)
    }
}

impl EventListener for OpenwireTraceListener {
    fn call_start(&self, ctx: &CallContext, _request: &Request<RequestBody>) {
        tracing::info!(
            target: "minos_mobile::network",
            component = self.component,
            call_id = %ctx.call_id(),
            dial_attempt = self.current_dial_attempt(),
            retry_count = self.retry_count(),
            redirect_count = self.redirect_count(),
            method = %self.method,
            uri = %self.uri,
            "openwire call start"
        );
    }

    fn call_end(&self, ctx: &CallContext) {
        tracing::info!(
            target: "minos_mobile::network",
            component = self.component,
            call_id = %ctx.call_id(),
            dial_attempt = self.current_dial_attempt(),
            retry_count = self.retry_count(),
            redirect_count = self.redirect_count(),
            method = %self.method,
            uri = %self.uri,
            "openwire call complete"
        );
    }

    fn call_failed(&self, ctx: &CallContext, error: &WireError) {
        log_wire_error(
            self.component,
            ctx,
            &self.method,
            &self.uri,
            "openwire call failed",
            error,
            None,
            None,
            None,
        );
    }

    fn dns_start(&self, ctx: &CallContext, host: &str, port: u16) {
        let dial_attempt = self.next_dial_attempt();
        tracing::info!(
            target: "minos_mobile::network",
            component = self.component,
            call_id = %ctx.call_id(),
            dial_attempt,
            retry_count = self.retry_count(),
            redirect_count = self.redirect_count(),
            method = %self.method,
            uri = %self.uri,
            host,
            port,
            "openwire dns start"
        );
    }

    fn dns_end(&self, ctx: &CallContext, host: &str, addrs: &[SocketAddr]) {
        let resolved = addrs
            .iter()
            .map(SocketAddr::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        tracing::info!(
            target: "minos_mobile::network",
            component = self.component,
            call_id = %ctx.call_id(),
            dial_attempt = self.current_dial_attempt(),
            retry_count = self.retry_count(),
            redirect_count = self.redirect_count(),
            method = %self.method,
            uri = %self.uri,
            host,
            resolved = %resolved,
            "openwire dns end"
        );
    }

    fn dns_failed(&self, ctx: &CallContext, host: &str, error: &WireError) {
        log_wire_error(
            self.component,
            ctx,
            &self.method,
            &self.uri,
            "openwire dns failed",
            error,
            None,
            None,
            Some(host),
        );
    }

    fn route_plan(&self, ctx: &CallContext, route_count: usize, fast_fallback_enabled: bool) {
        tracing::info!(
            target: "minos_mobile::network",
            component = self.component,
            call_id = %ctx.call_id(),
            dial_attempt = self.current_dial_attempt(),
            retry_count = self.retry_count(),
            redirect_count = self.redirect_count(),
            method = %self.method,
            uri = %self.uri,
            route_count,
            fast_fallback_enabled,
            "openwire route plan"
        );
    }

    fn connect_start(&self, ctx: &CallContext, addr: SocketAddr) {
        tracing::info!(
            target: "minos_mobile::network",
            component = self.component,
            call_id = %ctx.call_id(),
            dial_attempt = self.current_dial_attempt(),
            retry_count = self.retry_count(),
            redirect_count = self.redirect_count(),
            method = %self.method,
            uri = %self.uri,
            addr = %addr,
            "openwire connect start"
        );
    }

    fn connect_end(&self, ctx: &CallContext, connection_id: ConnectionId, addr: SocketAddr) {
        tracing::info!(
            target: "minos_mobile::network",
            component = self.component,
            call_id = %ctx.call_id(),
            dial_attempt = self.current_dial_attempt(),
            retry_count = self.retry_count(),
            redirect_count = self.redirect_count(),
            connection_id = %connection_id,
            method = %self.method,
            uri = %self.uri,
            addr = %addr,
            "openwire connect complete"
        );
    }

    fn connect_failed(&self, ctx: &CallContext, addr: SocketAddr, error: &WireError) {
        log_wire_error(
            self.component,
            ctx,
            &self.method,
            &self.uri,
            "openwire connect failed",
            error,
            Some(addr),
            None,
            None,
        );
    }

    fn tls_start(&self, ctx: &CallContext, server_name: &str) {
        tracing::info!(
            target: "minos_mobile::network",
            component = self.component,
            call_id = %ctx.call_id(),
            dial_attempt = self.current_dial_attempt(),
            retry_count = self.retry_count(),
            redirect_count = self.redirect_count(),
            method = %self.method,
            uri = %self.uri,
            server_name,
            "openwire tls start"
        );
    }

    fn tls_end(&self, ctx: &CallContext, server_name: &str) {
        tracing::info!(
            target: "minos_mobile::network",
            component = self.component,
            call_id = %ctx.call_id(),
            dial_attempt = self.current_dial_attempt(),
            retry_count = self.retry_count(),
            redirect_count = self.redirect_count(),
            method = %self.method,
            uri = %self.uri,
            server_name,
            "openwire tls complete"
        );
    }

    fn tls_failed(&self, ctx: &CallContext, server_name: &str, error: &WireError) {
        log_wire_error(
            self.component,
            ctx,
            &self.method,
            &self.uri,
            "openwire tls failed",
            error,
            None,
            Some(server_name),
            None,
        );
    }

    fn connect_race_start(
        &self,
        ctx: &CallContext,
        race_id: u64,
        route_index: usize,
        route_count: usize,
        route_family: &str,
    ) {
        tracing::info!(
            target: "minos_mobile::network",
            component = self.component,
            call_id = %ctx.call_id(),
            dial_attempt = self.current_dial_attempt(),
            retry_count = self.retry_count(),
            redirect_count = self.redirect_count(),
            race_id,
            route_index,
            route_count,
            route_family,
            method = %self.method,
            uri = %self.uri,
            "openwire connect race start"
        );
    }

    fn connect_race_won(
        &self,
        ctx: &CallContext,
        race_id: u64,
        route_index: usize,
        route_count: usize,
    ) {
        tracing::info!(
            target: "minos_mobile::network",
            component = self.component,
            call_id = %ctx.call_id(),
            dial_attempt = self.current_dial_attempt(),
            retry_count = self.retry_count(),
            redirect_count = self.redirect_count(),
            race_id,
            route_index,
            route_count,
            method = %self.method,
            uri = %self.uri,
            "openwire connect race won"
        );
    }

    fn connect_race_lost(
        &self,
        ctx: &CallContext,
        race_id: u64,
        route_index: usize,
        route_count: usize,
        reason: &str,
    ) {
        tracing::warn!(
            target: "minos_mobile::network",
            component = self.component,
            call_id = %ctx.call_id(),
            dial_attempt = self.current_dial_attempt(),
            retry_count = self.retry_count(),
            redirect_count = self.redirect_count(),
            race_id,
            route_index,
            route_count,
            reason,
            method = %self.method,
            uri = %self.uri,
            "openwire connect race lost"
        );
    }

    fn response_headers_end(&self, ctx: &CallContext, response: &Response<ResponseBody>) {
        tracing::info!(
            target: "minos_mobile::network",
            component = self.component,
            call_id = %ctx.call_id(),
            dial_attempt = self.current_dial_attempt(),
            retry_count = self.retry_count(),
            redirect_count = self.redirect_count(),
            method = %self.method,
            uri = %self.uri,
            status = %response.status(),
            "openwire response headers"
        );
    }

    fn retry(&self, ctx: &CallContext, attempt: u32, reason: &str) {
        self.retry_count.store(attempt, Ordering::Relaxed);
        tracing::warn!(
            target: "minos_mobile::network",
            component = self.component,
            call_id = %ctx.call_id(),
            dial_attempt = self.current_dial_attempt(),
            retry_count = attempt,
            redirect_count = self.redirect_count(),
            next_dial_attempt = self.current_dial_attempt() + 1,
            reason,
            method = %self.method,
            uri = %self.uri,
            "openwire retry scheduled"
        );
    }

    fn redirect(&self, ctx: &CallContext, attempt: u32, location: &http::Uri) {
        self.redirect_count.store(attempt, Ordering::Relaxed);
        tracing::info!(
            target: "minos_mobile::network",
            component = self.component,
            call_id = %ctx.call_id(),
            dial_attempt = self.current_dial_attempt(),
            retry_count = self.retry_count(),
            redirect_count = attempt,
            method = %self.method,
            uri = %self.uri,
            location = %location,
            "openwire redirect scheduled"
        );
    }
}
