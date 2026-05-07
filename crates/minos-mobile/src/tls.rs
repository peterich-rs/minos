use std::sync::Once;

use openwire::{RustlsTlsConnector, WireError};
#[cfg(target_os = "android")]
use rustls::{ClientConfig, RootCertStore};
#[cfg(target_os = "android")]
use webpki_roots::TLS_SERVER_ROOTS;

static INSTALL_RUSTLS_PROVIDER: Once = Once::new();

pub(crate) fn install_default_rustls_provider() {
    INSTALL_RUSTLS_PROVIDER.call_once(|| {
        let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    });
}

pub(crate) fn build_mobile_tls_connector() -> Result<RustlsTlsConnector, WireError> {
    install_default_rustls_provider();

    #[cfg(target_os = "android")]
    {
        let roots = TLS_SERVER_ROOTS.iter().cloned().collect::<RootCertStore>();

        tracing::info!(
            target: "minos_mobile::tls",
            tls_backend = "rustls",
            verifier_backend = "webpki-roots",
            target_os = std::env::consts::OS,
            custom_root_count = roots.len(),
            "configured mobile TLS verifier"
        );

        let config = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();

        return Ok(RustlsTlsConnector::from_config(config));
    }

    let builder = RustlsTlsConnector::builder();

    builder.build()
}

pub(crate) fn build_mobile_websocket_tls_connector() -> Result<RustlsTlsConnector, WireError> {
    install_default_rustls_provider();

    #[cfg(target_os = "android")]
    {
        let roots = TLS_SERVER_ROOTS.iter().cloned().collect::<RootCertStore>();

        tracing::info!(
            target: "minos_mobile::tls",
            tls_backend = "rustls",
            verifier_backend = "webpki-roots",
            target_os = std::env::consts::OS,
            alpn = "http/1.1",
            custom_root_count = roots.len(),
            "configured mobile websocket TLS verifier"
        );

        let mut config = ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth();
        config.alpn_protocols = websocket_alpn_protocols();

        return Ok(RustlsTlsConnector::from_config(config));
    }

    build_mobile_tls_connector()
}

#[cfg(any(target_os = "android", test))]
fn websocket_alpn_protocols() -> Vec<Vec<u8>> {
    vec![b"http/1.1".to_vec()]
}

#[cfg(test)]
mod tests {
    use super::{
        build_mobile_tls_connector, build_mobile_websocket_tls_connector,
        install_default_rustls_provider, websocket_alpn_protocols,
    };

    #[test]
    fn installs_default_rustls_provider() {
        install_default_rustls_provider();
        assert!(rustls::crypto::CryptoProvider::get_default().is_some());
    }

    #[test]
    fn builds_mobile_tls_connector() {
        build_mobile_tls_connector().expect("mobile TLS connector should build");
    }

    #[test]
    fn builds_mobile_websocket_tls_connector() {
        build_mobile_websocket_tls_connector().expect("mobile websocket TLS connector should build");
    }

    #[test]
    fn websocket_alpn_prefers_http1_only() {
        assert_eq!(websocket_alpn_protocols(), vec![b"http/1.1".to_vec()]);
    }
}
