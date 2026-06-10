//! darkrun-vcs — OAuth + version-control providers for the darkrun factory.
//!
//! The **external** Checkpoint kind hands a Pass off to a human via a Pull
//! Request (GitHub) or Merge Request (GitLab). This crate is what turns that
//! handoff into a real change request, and what authenticates the operator to
//! the provider in the first place.
//!
//! Everything that touches the network goes through one injectable seam, the
//! [`HttpTransport`] trait, so the whole crate is testable offline with
//! [`MockTransport`] and carries no HTTP client dependency of its own. The
//! binary wires a real adapter at the edge.
//!
//! ## Shape
//!
//! - [`Provider`] — GitHub / GitLab, with their OAuth and REST endpoints.
//! - [`oauth`] — build the authorize URL ([`oauth::authorize_url`]) and run the
//!   server-side code→token exchange ([`oauth::exchange_code`]).
//! - [`Credential`] + [`CredentialStore`] — the OAuth token model and its
//!   `~/.darkrun/credentials` persistence (`0600` on unix).
//! - [`RepoCoords`] + [`parse_remote_url`] — host/owner/repo from a git remote.
//! - [`rest`] — GitHub PR / GitLab MR clients and the unified
//!   [`create_change_request`].
//!
//! ## OAuth model
//!
//! The website hosts the OAuth dance and holds the client secrets; the CLI
//! brokers an authorization-code flow against it. This crate provides the
//! pieces both sides need: the CLI/website build the authorize URL, the website
//! performs the exchange, and the CLI persists the resulting [`Credential`].

#![deny(missing_docs)]

pub mod error;
pub mod oauth;
pub mod provider;
pub mod remote;
pub mod rest;
pub mod store;
pub mod transport;

pub use error::{Result, VcsError};
pub use oauth::{authorize_url, exchange_code, percent_encode};
pub use provider::{Credential, Provider};
pub use remote::{parse_remote_url, RepoCoords};
pub use rest::{
    create_change_request, github_create_comment, github_create_pull_request,
    github_create_pull_request_with, github_get_repo, github_mark_ready, github_pull_view,
    github_review_notes, gitlab_create_merge_request, gitlab_create_note, gitlab_mark_ready,
    gitlab_mr_view, gitlab_notes, gitlab_resolve_project, ChangeRequest, ChangeRequestState,
    ChangeRequestView, RemoteNote, RepoInfo,
};
pub use store::CredentialStore;
pub use transport::{HttpRequest, HttpResponse, HttpTransport, Method, MockTransport};
