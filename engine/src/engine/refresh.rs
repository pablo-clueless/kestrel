//! Token refresh during load runs (HANDOFF → Open questions 3): long runs, soaks above all, outlive
//! short-lived tokens, which used to show up as a wall of 401s.
//!
//! The token comes from an ordinary endpoint whose On Response rules pick it out of the response,
//! the same rules Send applies. The refresher sends that endpoint at the start of the run, every
//! `every`, and early when the run starts getting 401s; then it compiles the run's requests again
//! with the new value, and the load runner swaps them in without pausing. A rule that saves a
//! secret stores it like Send does, so the next run starts with it too. A rule that saves a
//! variable only applies to this run: variables belong to the UI, which saves the workspace and
//! would overwrite one changed behind its back.

use std::{collections::BTreeMap, sync::Arc, time::Duration};

use super::client::{self, ClientOptions};
use crate::{
    extract,
    model::{Endpoint, ExtractTarget, Workspace, store::WorkspaceStore},
    template::request::CompiledRequest,
};

/// Least time between two refreshes, however many 401s arrive (shorter in tests, to keep them quick).
pub const MIN_GAP: Duration = if cfg!(test) { Duration::from_millis(500) } else { Duration::from_secs(5) };

pub struct Refresher {
    pub every: Duration,
    /// The endpoint that gets a token; it has enabled On Response rules.
    pub token_endpoint: Endpoint,
    /// The run's endpoints, in the order the load runner sends them (one, or the mix's order).
    pub targets: Vec<Endpoint>,
    pub environment: String,
    pub store: Arc<WorkspaceStore>,
    pub timeout: Duration,
    /// Variables picked out so far, applied on top of the environment for this run only.
    vars: BTreeMap<String, String>,
}

impl Refresher {
    pub fn new(
        every: Duration,
        token_endpoint: Endpoint,
        targets: Vec<Endpoint>,
        environment: String,
        store: Arc<WorkspaceStore>,
        timeout: Duration,
    ) -> Self {
        Self { every, token_endpoint, targets, environment, store, timeout, vars: BTreeMap::new() }
    }

    pub fn name(&self) -> &str {
        if self.token_endpoint.name.is_empty() { &self.token_endpoint.url } else { &self.token_endpoint.name }
    }

    /// Sends the token request, saves what its rules pick out, and returns the run's requests
    /// compiled again with it (in `targets` order).
    pub async fn refresh(&mut self) -> Result<Vec<CompiledRequest>, String> {
        let token_request = self.compile(&self.token_endpoint.clone()).await?;
        let rendered = token_request.render()?;
        let opts =
            ClientOptions { keep_alive: false, follow_redirects: true, confirmed_hosts: self.store.confirmed_hosts() };
        let target = client::connect(&rendered.url, &opts).await?;
        let outcome = client::execute(&target.client, &rendered, self.timeout).await;
        let status = match (outcome.status, &outcome.error) {
            (Some(status), _) => status,
            (None, Some((_, message))) => return Err(format!("no response ({message})")),
            (None, None) => return Err("no response".into()),
        };
        if !(200..300).contains(&status) {
            return Err(format!("the token request got {status}"));
        }
        let mut saved = 0;
        for rule in self.token_endpoint.extract.iter().filter(|r| r.enabled && !r.name.trim().is_empty()) {
            let value = extract::pick(rule, status, &outcome.response_headers, &outcome.body)?;
            let name = rule.name.trim();
            match rule.target {
                ExtractTarget::Secret => self
                    .store
                    .set_secret(&self.environment, name, Some(value))
                    .await
                    .map_err(|e| format!("couldn't save `{name}`: {e:#}"))?,
                ExtractTarget::Variable => {
                    self.vars.insert(name.to_owned(), value);
                }
            }
            saved += 1;
        }
        if saved == 0 {
            return Err("its On Response rules saved nothing".into());
        }
        let mut compiled = Vec::with_capacity(self.targets.len());
        for endpoint in self.targets.clone() {
            compiled.push(self.compile(&endpoint).await?);
        }
        Ok(compiled)
    }

    async fn compile(&self, endpoint: &Endpoint) -> Result<CompiledRequest, String> {
        let files = self.store.files_for(endpoint).await.map_err(|e| format!("{e:#}"))?;
        CompiledRequest::compile_with(
            endpoint,
            &self.workspace(),
            &self.store.secrets(),
            &files,
            Some(&self.environment),
            false,
        )
        .map_err(|e| e.to_string())
    }

    /// The saved workspace, with this run's refreshed variables on top of the environment.
    fn workspace(&self) -> Workspace {
        let mut ws = self.store.workspace();
        if let Some(env) = ws.environments.iter_mut().find(|e| e.name == self.environment) {
            env.vars.extend(self.vars.clone());
        }
        ws
    }
}

/// What the refresher did, for the report's notes.
#[derive(Debug, Default)]
pub struct Tally {
    pub refreshed: u32,
    /// Of those, how many were early because of 401s.
    pub after_401s: u32,
    pub failures: Vec<String>,
}

impl Tally {
    pub fn note(&self, name: &str, every: Duration) -> Vec<String> {
        let mut notes = Vec::new();
        let minutes = every.as_secs_f64() / 60.0;
        let every = if minutes >= 1.0 { format!("{minutes:.0} min") } else { format!("{} s", every.as_secs()) };
        if self.refreshed > 0 {
            let early = match self.after_401s {
                0 => String::new(),
                n => format!(", {n} of them early because of 401s"),
            };
            notes.push(format!(
                "Refreshed the token with `{name}` {} time{} (every {every}{early}).",
                self.refreshed,
                if self.refreshed == 1 { "" } else { "s" }
            ));
        }
        if let Some(first) = self.failures.first() {
            notes.push(format!(
                "The token request `{name}` failed {} time{} ({first}); the run kept the token it had.",
                self.failures.len(),
                if self.failures.len() == 1 { "" } else { "s" }
            ));
        }
        notes
    }
}
