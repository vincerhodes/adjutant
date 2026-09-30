//! SMTP send: the mockable trait plus the live lettre 0.11 (rustls) impl.
//! Sync API — lettre's async transports stay disabled (no tokio/async-std).

use std::str::FromStr;

use crate::email::{sanitize_smtp_error, EmailError};

/// What the sync engine needs from SMTP. Mocked in tests.
pub trait SmtpTransport {
    /// Send a fully-formed RFC822 message.
    fn send_rfc822(
        &mut self,
        from: &str,
        to: &[String],
        cc: &[String],
        bcc: &[String],
        rfc822: &[u8],
    ) -> Result<(), EmailError>;
}

/// Live SMTP client (submission, rustls).
pub struct LiveSmtp {
    transport: lettre::SmtpTransport,
}

impl LiveSmtp {
    pub fn connect(
        host: &str,
        port: u16,
        username: &str,
        password: &str,
    ) -> Result<Self, EmailError> {
        use lettre::transport::smtp::authentication::Credentials;
        let transport = lettre::SmtpTransport::relay(host)
            .map_err(|e| EmailError::Smtp(format!("{e:?}")))?
            .port(port)
            .credentials(Credentials::new(username.to_string(), password.to_string()))
            .build();
        // Force the connection + auth now so "test connection" fails fast.
        transport
            .test_connection()
            .map_err(|e| EmailError::Smtp(sanitize_smtp_error(&e)))?;
        Ok(LiveSmtp { transport })
    }
}

impl SmtpTransport for LiveSmtp {
    fn send_rfc822(
        &mut self,
        from: &str,
        to: &[String],
        cc: &[String],
        bcc: &[String],
        rfc822: &[u8],
    ) -> Result<(), EmailError> {
        use lettre::Transport;
        let parse = |s: &str| {
            lettre::Address::from_str(s).map_err(|e| EmailError::Smtp(format!("address {s}: {e}")))
        };
        let from_addr = parse(from)?;
        let all: Vec<String> = to
            .iter()
            .chain(cc.iter())
            .chain(bcc.iter())
            .cloned()
            .collect();
        let to_addrs: Vec<lettre::Address> =
            all.iter().map(|s| parse(s)).collect::<Result<_, _>>()?;
        let envelope = lettre::address::Envelope::new(Some(from_addr), to_addrs)
            .map_err(|e| EmailError::Smtp(e.to_string()))?;
        self.transport
            .send_raw(&envelope, rfc822)
            .map_err(|e| EmailError::Smtp(sanitize_smtp_error(&e)))?;
        Ok(())
    }
}
