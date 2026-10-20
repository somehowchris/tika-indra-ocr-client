//! Shared HTTP client policy for frozen-source acquisition.
//!
//! Publishers can reject anonymous clients. The hosted release run
//! 39364554907 received HTTP 403 from scielo.br for `sources/hops.pdf`
//! because the evaluator sent no `User-Agent` header. Every acquisition
//! request now identifies this evaluator and its version explicitly; the
//! frozen size and SHA-256 checks still decide whether a body is accepted.
use std::time::Duration;

/// Product token sent with every acquisition request.
pub static USER_AGENT: &str = concat!("indra-evaluate/", env!("CARGO_PKG_VERSION"));

/// Identification and finite deadlines shared by every acquisition client.
pub fn client_builder() -> reqwest::blocking::ClientBuilder {
    reqwest::blocking::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(20))
        .timeout(Duration::from_secs(180))
}

/// The production acquisition client: identified, bounded and HTTPS-only.
pub fn acquisition_client() -> reqwest::Result<reqwest::blocking::Client> {
    client_builder().https_only(true).build()
}

#[cfg(test)]
mod tests {
    use super::{USER_AGENT, acquisition_client, client_builder};
    use std::{
        io::{Read, Write},
        net::TcpListener,
        thread,
    };

    #[test]
    fn user_agent_names_the_evaluator_and_its_package_version() {
        assert_eq!(
            USER_AGENT,
            format!("indra-evaluate/{}", env!("CARGO_PKG_VERSION"))
        );
        let version = USER_AGENT.strip_prefix("indra-evaluate/").unwrap();
        assert!(version.split('.').count() >= 3, "{USER_AGENT}");
        assert!(version.chars().all(|c| c.is_ascii_digit() || c == '.'));
    }

    #[test]
    fn builder_sends_the_explicit_user_agent_on_the_wire() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let peer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut head = Vec::new();
            let mut buffer = [0_u8; 4096];
            loop {
                let read = stream.read(&mut buffer).unwrap();
                head.extend_from_slice(&buffer[..read]);
                if read == 0 || head.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            stream
                .write_all(
                    b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
            String::from_utf8(head).unwrap()
        });
        let client = client_builder().build().unwrap();
        let response = client
            .get(format!("http://{address}/probe"))
            .send()
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);
        let head = peer.join().unwrap();
        let agents: Vec<&str> = head
            .lines()
            .filter_map(|line| line.split_once(':'))
            .filter(|(name, _)| name.eq_ignore_ascii_case("user-agent"))
            .map(|(_, value)| value.trim())
            .collect();
        assert_eq!(agents, [USER_AGENT], "{head}");
    }

    #[test]
    fn acquisition_client_refuses_plaintext_sources_before_connecting() {
        let client = acquisition_client().unwrap();
        let error = client.get("http://127.0.0.1:9/").send().unwrap_err();
        assert!(error.is_builder(), "{error}");
        assert!(!error.is_connect(), "{error}");
    }
}
