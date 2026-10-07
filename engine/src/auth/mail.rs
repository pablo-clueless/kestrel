//! Account emails (A3): email verification and password reset, sent over SMTP. Plain text only:
//! one link and a sentence or two, which every client renders and spam filters don't mind.

use std::time::Duration;

use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    message::{Mailbox, header::ContentType},
    transport::smtp::authentication::Credentials,
};

use crate::config::{SmtpConfig, SmtpTls};

/// One email to send.
#[derive(Debug, Clone)]
pub struct Email {
    pub to: String,
    pub subject: String,
    pub body: String,
}

pub enum Mailer {
    Smtp {
        // Boxed: it's large next to the test-only capture variant.
        transport: Box<AsyncSmtpTransport<Tokio1Executor>>,
        from: Mailbox,
    },
    /// Keeps what would have been sent, so tests can follow the link in it.
    #[cfg(test)]
    Capture(std::sync::Mutex<Vec<Email>>),
}

impl Mailer {
    pub fn smtp(config: &SmtpConfig) -> anyhow::Result<Self> {
        let from: Mailbox = config
            .from
            .parse()
            .map_err(|e| anyhow::anyhow!("KESTREL_SMTP_FROM isn't a valid address ({e}): {}", config.from))?;
        let builder = match config.tls {
            SmtpTls::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&config.host)?,
            SmtpTls::StartTls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&config.host)?,
            SmtpTls::None => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&config.host),
        };
        let mut builder = builder.port(config.port).timeout(Some(Duration::from_secs(15)));
        if let (Some(user), Some(pass)) = (&config.username, &config.password) {
            builder = builder.credentials(Credentials::new(user.clone(), pass.clone()));
        }
        Ok(Self::Smtp { transport: Box::new(builder.build()), from })
    }

    pub async fn send(&self, email: Email) -> anyhow::Result<()> {
        match self {
            Self::Smtp { transport, from } => {
                let to: Mailbox = email.to.parse().map_err(|e| anyhow::anyhow!("bad recipient {}: {e}", email.to))?;
                let message = Message::builder()
                    .from(from.clone())
                    .to(to)
                    .subject(email.subject)
                    .header(ContentType::TEXT_PLAIN)
                    .body(email.body)?;
                transport.send(message).await?;
                Ok(())
            }
            #[cfg(test)]
            Self::Capture(sent) => {
                sent.lock().unwrap().push(email);
                Ok(())
            }
        }
    }
}

#[cfg(test)]
impl Mailer {
    pub fn capture() -> Self {
        Self::Capture(std::sync::Mutex::default())
    }

    /// What's been sent so far.
    pub fn sent(&self) -> Vec<Email> {
        match self {
            Self::Capture(sent) => sent.lock().unwrap().clone(),
            Self::Smtp { .. } => Vec::new(),
        }
    }
}

pub fn verification(to: &str, link: &str) -> Email {
    Email {
        to: to.to_owned(),
        subject: "Confirm your email for Kestrel".into(),
        body: format!(
            "Confirm that this is your email address by opening this link:\n\n{link}\n\n\
             The link works once and expires in 24 hours. If you didn't create a Kestrel account, \
             you can ignore this email.\n"
        ),
    }
}

/// The subject stays fixed: the workspace name is chosen by whoever invites, so it only goes in
/// the body, next to who sent it.
/// `access` reads after the workspace name: "as an admin", "with write access", "with read access".
pub fn invite(to: &str, inviter: &str, workspace: &str, access: &str, link: &str) -> Email {
    Email {
        to: to.to_owned(),
        subject: "You're invited to a Kestrel workspace".into(),
        body: format!(
            "{inviter} invited you to the Kestrel workspace \"{workspace}\" {access}. To join, open \
             this link and sign in (or create an account) with this email address:\n\n{link}\n\n\
             The link works once and expires in 7 days. If you weren't expecting this, you can \
             ignore this email.\n"
        ),
    }
}

pub fn password_reset(to: &str, link: &str) -> Email {
    Email {
        to: to.to_owned(),
        subject: "Reset your Kestrel password".into(),
        body: format!(
            "Someone asked to reset the password for this Kestrel account. To choose a new one, open \
             this link:\n\n{link}\n\nThe link works once and expires in 30 minutes. Setting a new \
             password signs you out everywhere. If it wasn't you, ignore this email; your password \
             stays the same.\n"
        ),
    }
}
