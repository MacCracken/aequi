use chrono::NaiveDate;
use rust_decimal::prelude::{FromPrimitive, ToPrimitive};
use rust_decimal::{Decimal, RoundingStrategy};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;

const GITHUB_API: &str = "https://api.github.com";
const LINEAR_API: &str = "https://api.linear.app/graphql";
/// Upper bound on paginated requests (100 items each) per fetch.
const MAX_PAGES: usize = 50;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Source from which to fetch completed work items.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WorkItemSource {
    GitHub {
        owner: String,
        repo: String,
        token: String,
    },
    Linear {
        api_key: String,
        team_id: String,
        /// Linear estimates are story points; each point is billed as this
        /// many hours.
        #[serde(default = "default_hours_per_point")]
        hours_per_point: f64,
    },
}

fn default_hours_per_point() -> f64 {
    1.0
}

fn validate_hours_per_point(hours_per_point: f64) -> Result<(), WorkItemError> {
    if hours_per_point.is_finite() && hours_per_point > 0.0 {
        Ok(())
    } else {
        Err(WorkItemError::InvalidInput(
            "hours_per_point must be a positive number".into(),
        ))
    }
}

/// A single completed work item (issue / PR) that may become an invoice line.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkItem {
    pub id: String,
    pub title: String,
    pub url: String,
    pub completed_at: Option<NaiveDate>,
    pub labels: Vec<String>,
    pub milestone: Option<String>,
    pub assignee: Option<String>,
    /// Estimated hours spent. `None` means unknown (will default to 1 when
    /// converting to an invoice line estimate).
    pub hours: Option<f64>,
}

/// Filters applied when fetching work items.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WorkItemFilter {
    /// Milestone title. For GitHub a title is resolved to its milestone number;
    /// a purely numeric value that matches no title is used as the number, and
    /// `*` / `none` are passed through.
    pub milestone: Option<String>,
    pub label: Option<String>,
    /// Only items completed (closed) on or after this date.
    pub since: Option<NaiveDate>,
    pub assignee: Option<String>,
}

/// A prospective invoice line derived from a [`WorkItem`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InvoiceLineEstimate {
    pub description: String,
    pub hours: Decimal,
    pub rate_cents: i64,
    pub total_cents: i64,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum WorkItemError {
    #[error("HTTP request failed: {0}")]
    RequestFailed(String),
    #[error("failed to parse response: {0}")]
    ParseError(String),
    #[error("invalid work item source or filter: {0}")]
    InvalidInput(String),
}

// ---------------------------------------------------------------------------
// GitHub response types (internal)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct GitHubIssue {
    number: i64,
    title: String,
    html_url: String,
    closed_at: Option<String>,
    labels: Vec<GitHubLabel>,
    milestone: Option<GitHubMilestone>,
    assignee: Option<GitHubAssignee>,
    #[serde(default)]
    assignees: Vec<GitHubAssignee>,
    state_reason: Option<String>,
    /// Present when the "issue" is a pull request.
    pull_request: Option<GitHubPullRequestRef>,
}

impl GitHubIssue {
    /// Closed as done: not "not planned", and for pull requests, merged.
    fn is_completed(&self) -> bool {
        self.state_reason.as_deref() != Some("not_planned")
            && self
                .pull_request
                .as_ref()
                .is_none_or(|pr| pr.merged_at.is_some())
    }
}

#[derive(Debug, Deserialize)]
struct GitHubLabel {
    name: String,
}

#[derive(Debug, Deserialize)]
struct GitHubMilestone {
    title: String,
}

#[derive(Debug, Deserialize)]
struct GitHubMilestoneRef {
    number: i64,
    title: String,
}

#[derive(Debug, Deserialize)]
struct GitHubAssignee {
    login: String,
}

#[derive(Debug, Deserialize)]
struct GitHubPullRequestRef {
    merged_at: Option<String>,
}

// ---------------------------------------------------------------------------
// Linear response types (internal)
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct LinearResponse {
    data: Option<LinearData>,
    errors: Option<Vec<LinearError>>,
}

#[derive(Debug, Deserialize)]
struct LinearData {
    issues: LinearIssueConnection,
}

#[derive(Debug, Deserialize)]
struct LinearIssueConnection {
    nodes: Vec<LinearIssue>,
    #[serde(rename = "pageInfo")]
    page_info: Option<LinearPageInfo>,
}

#[derive(Debug, Deserialize)]
struct LinearPageInfo {
    #[serde(rename = "hasNextPage")]
    has_next_page: bool,
    #[serde(rename = "endCursor")]
    end_cursor: Option<String>,
}

#[derive(Debug, Deserialize)]
struct LinearIssue {
    identifier: String,
    title: String,
    url: String,
    #[serde(rename = "completedAt")]
    completed_at: Option<String>,
    labels: LinearLabelConnection,
    assignee: Option<LinearAssignee>,
    #[serde(rename = "projectMilestone")]
    project_milestone: Option<LinearMilestone>,
    estimate: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct LinearLabelConnection {
    nodes: Vec<LinearLabel>,
}

#[derive(Debug, Deserialize)]
struct LinearLabel {
    name: String,
}

#[derive(Debug, Deserialize)]
struct LinearAssignee {
    name: String,
}

#[derive(Debug, Deserialize)]
struct LinearMilestone {
    name: String,
}

#[derive(Debug, Deserialize)]
struct LinearError {
    message: String,
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Fetch completed work items from the given source, applying the filter.
pub async fn fetch_work_items(
    source: &WorkItemSource,
    filter: &WorkItemFilter,
) -> Result<Vec<WorkItem>, WorkItemError> {
    match source {
        WorkItemSource::GitHub { owner, repo, token } => {
            fetch_github_issues(owner, repo, token, filter).await
        }
        WorkItemSource::Linear {
            api_key,
            team_id,
            hours_per_point,
        } => {
            validate_hours_per_point(*hours_per_point)?;
            fetch_linear_issues(api_key, team_id, *hours_per_point, filter).await
        }
    }
}

/// Convert work items into invoice line estimates at the given hourly rate.
///
/// Items without an `hours` value default to 1 hour. Hours are kept to four
/// decimal places and the line total is rounded half-up to whole cents.
pub fn estimate_line_items(items: &[WorkItem], hourly_rate_cents: i64) -> Vec<InvoiceLineEstimate> {
    items
        .iter()
        .map(|item| {
            // `from_f64` (unlike `from_f64_retain`) drops binary noise: 0.3 → 0.3,
            // not 0.29999999999999998889776975.
            let hours = item
                .hours
                .and_then(Decimal::from_f64)
                .map(|h| h.round_dp(4).normalize())
                .unwrap_or(Decimal::ONE);
            let total_cents = hours
                .checked_mul(Decimal::from(hourly_rate_cents))
                .map(|t| t.round_dp_with_strategy(0, RoundingStrategy::MidpointAwayFromZero))
                .and_then(|t| t.to_i64())
                .unwrap_or(0);

            InvoiceLineEstimate {
                description: item.title.clone(),
                hours,
                rate_cents: hourly_rate_cents,
                total_cents,
            }
        })
        .collect()
}

fn http_client() -> Result<reqwest::Client, WorkItemError> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .map_err(|e| WorkItemError::RequestFailed(e.to_string()))
}

/// Parse a date-time prefix like `2026-03-10T15:30:00Z` into its date.
fn date_prefix(s: &str) -> Option<NaiveDate> {
    s.get(..10)
        .and_then(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").ok())
}

/// Keep items completed on or after `since` (items without a completion date
/// are dropped when a cutoff is given).
fn retain_completed_since(items: &mut Vec<WorkItem>, since: Option<NaiveDate>) {
    if let Some(since) = since {
        items.retain(|item| item.completed_at.is_some_and(|d| d >= since));
    }
}

// ---------------------------------------------------------------------------
// GitHub fetcher
// ---------------------------------------------------------------------------

/// Percent-encode a query value, leaving only unreserved characters as-is.
fn url_encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'0'..=b'9' | b'a'..=b'z' | b'A'..=b'Z' | b'-' | b'_' | b'.' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// GitHub owner / repository names: letters, digits, `-`, `_`, `.`.
/// Rejecting anything else keeps the values from rewriting the API path.
fn is_valid_github_name(name: &str) -> bool {
    !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

/// Extract the `rel="next"` target from a GitHub `Link` header.
fn next_link(header: &str) -> Option<String> {
    header.split(',').find_map(|part| {
        let (url, params) = part.trim().split_once(';')?;
        let url = url.trim().strip_prefix('<')?.strip_suffix('>')?;
        let is_next = params.split(';').any(|param| {
            param.trim().strip_prefix("rel=").is_some_and(|rel| {
                rel.trim_matches('"')
                    .split_whitespace()
                    .any(|r| r == "next")
            })
        });
        is_next.then(|| url.to_string())
    })
}

/// GET every page of a GitHub list endpoint by following `Link: rel="next"`.
async fn github_get_all<T: DeserializeOwned>(
    client: &reqwest::Client,
    first_url: String,
    token: &str,
) -> Result<Vec<T>, WorkItemError> {
    let mut out = Vec::new();
    let mut next = Some(first_url);
    let mut pages = 0;

    while let Some(url) = next.take() {
        pages += 1;
        if pages > MAX_PAGES {
            return Err(WorkItemError::RequestFailed(format!(
                "GitHub returned more than {MAX_PAGES} pages; narrow the filter"
            )));
        }

        let resp = client
            .get(&url)
            .header("Authorization", format!("Bearer {token}"))
            .header("User-Agent", "aequi-import")
            .header("Accept", "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| WorkItemError::RequestFailed(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(WorkItemError::RequestFailed(format!(
                "GitHub API returned {}",
                resp.status()
            )));
        }

        next = match resp
            .headers()
            .get(reqwest::header::LINK)
            .and_then(|v| v.to_str().ok())
            .and_then(next_link)
        {
            // Never send the token anywhere but the GitHub API.
            Some(link) if link.starts_with(&format!("{GITHUB_API}/")) => Some(link),
            Some(link) => {
                return Err(WorkItemError::ParseError(format!(
                    "unexpected pagination link: {link}"
                )));
            }
            None => None,
        };

        let page: Vec<T> = resp
            .json()
            .await
            .map_err(|e| WorkItemError::ParseError(e.to_string()))?;
        out.extend(page);
    }

    Ok(out)
}

/// Map a milestone filter to the value GitHub's `milestone` parameter expects
/// (a milestone number, `*` or `none`).
fn pick_github_milestone(
    milestone: &str,
    milestones: &[GitHubMilestoneRef],
) -> Result<String, WorkItemError> {
    let wanted = milestone.trim();
    if wanted == "*" || wanted.eq_ignore_ascii_case("none") {
        return Ok(wanted.to_ascii_lowercase());
    }
    if let Some(m) = milestones
        .iter()
        .find(|m| m.title.trim().eq_ignore_ascii_case(wanted))
    {
        return Ok(m.number.to_string());
    }
    if !wanted.is_empty() && wanted.bytes().all(|b| b.is_ascii_digit()) {
        return Ok(wanted.to_string());
    }
    Err(WorkItemError::InvalidInput(format!(
        "milestone '{wanted}' not found"
    )))
}

async fn fetch_github_issues(
    owner: &str,
    repo: &str,
    token: &str,
    filter: &WorkItemFilter,
) -> Result<Vec<WorkItem>, WorkItemError> {
    if !is_valid_github_name(owner) || !is_valid_github_name(repo) {
        return Err(WorkItemError::InvalidInput(format!(
            "invalid GitHub repository '{owner}/{repo}'"
        )));
    }
    let client = http_client()?;

    let mut url = format!("{GITHUB_API}/repos/{owner}/{repo}/issues?state=closed&per_page=100");

    if let Some(ref milestone) = filter.milestone {
        let trimmed = milestone.trim();
        let milestones: Vec<GitHubMilestoneRef> =
            if trimmed == "*" || trimmed.eq_ignore_ascii_case("none") {
                Vec::new()
            } else {
                github_get_all(
                    &client,
                    format!("{GITHUB_API}/repos/{owner}/{repo}/milestones?state=all&per_page=100"),
                    token,
                )
                .await?
            };
        let number = pick_github_milestone(milestone, &milestones)?;
        url.push_str(&format!("&milestone={}", url_encode(&number)));
    }
    if let Some(ref label) = filter.label {
        url.push_str(&format!("&labels={}", url_encode(label)));
    }
    if let Some(since) = filter.since {
        // GitHub's `since` is "updated at or after". Closing an issue updates
        // it, so this only narrows the download; the closed-at cutoff is
        // applied below.
        url.push_str(&format!("&since={since}T00:00:00Z"));
    }

    let issues: Vec<GitHubIssue> = github_get_all(&client, url, token).await?;

    let mut items = Vec::new();
    for issue in issues {
        if !issue.is_completed() {
            continue;
        }
        let assignee = issue.assignee.map(|a| a.login);
        // Client-side assignee filter, over every assignee of the issue.
        if let Some(ref wanted) = filter.assignee {
            let assigned = assignee
                .iter()
                .chain(issue.assignees.iter().map(|a| &a.login))
                .any(|login| login.eq_ignore_ascii_case(wanted));
            if !assigned {
                continue;
            }
        }
        items.push(WorkItem {
            id: issue.number.to_string(),
            title: issue.title,
            url: issue.html_url,
            completed_at: issue.closed_at.as_deref().and_then(date_prefix),
            labels: issue.labels.into_iter().map(|l| l.name).collect(),
            milestone: issue.milestone.map(|m| m.title),
            assignee,
            hours: None,
        });
    }

    retain_completed_since(&mut items, filter.since);
    Ok(items)
}

// ---------------------------------------------------------------------------
// Linear fetcher
// ---------------------------------------------------------------------------

/// Escape a value for a GraphQL string literal.
fn gql_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Build the paginated issues query. The cursor is passed as `$after`.
fn linear_query(team_id: &str, filter: &WorkItemFilter) -> String {
    let mut conditions = vec![format!(
        "team: {{ id: {{ eq: \"{}\" }} }}",
        gql_escape(team_id)
    )];

    // A single completedAt comparator: GraphQL rejects duplicate input fields.
    conditions.push(match filter.since {
        Some(since) => format!("completedAt: {{ gte: \"{since}T00:00:00.000Z\" }}"),
        None => "completedAt: { null: false }".to_string(),
    });

    if let Some(ref label) = filter.label {
        conditions.push(format!(
            "labels: {{ name: {{ eq: \"{}\" }} }}",
            gql_escape(label)
        ));
    }
    if let Some(ref assignee) = filter.assignee {
        conditions.push(format!(
            "assignee: {{ name: {{ eq: \"{}\" }} }}",
            gql_escape(assignee)
        ));
    }

    let filter_str = conditions.join(", ");

    format!(
        r#"query Issues($after: String) {{
  issues(filter: {{ {filter_str} }}, first: 100, after: $after) {{
    nodes {{
      identifier
      title
      url
      completedAt
      labels {{
        nodes {{
          name
        }}
      }}
      assignee {{
        name
      }}
      projectMilestone {{
        name
      }}
      estimate
    }}
    pageInfo {{
      hasNextPage
      endCursor
    }}
  }}
}}"#
    )
}

async fn fetch_linear_issues(
    api_key: &str,
    team_id: &str,
    hours_per_point: f64,
    filter: &WorkItemFilter,
) -> Result<Vec<WorkItem>, WorkItemError> {
    let client = http_client()?;
    let query = linear_query(team_id, filter);

    let mut nodes = Vec::new();
    let mut after: Option<String> = None;
    let mut pages = 0;

    loop {
        pages += 1;
        if pages > MAX_PAGES {
            return Err(WorkItemError::RequestFailed(format!(
                "Linear returned more than {MAX_PAGES} pages; narrow the filter"
            )));
        }

        let body = serde_json::json!({ "query": query, "variables": { "after": after } });

        let resp = client
            .post(LINEAR_API)
            .header("Authorization", api_key)
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| WorkItemError::RequestFailed(e.to_string()))?;

        if !resp.status().is_success() {
            return Err(WorkItemError::RequestFailed(format!(
                "Linear API returned {}",
                resp.status()
            )));
        }

        let linear_resp: LinearResponse = resp
            .json()
            .await
            .map_err(|e| WorkItemError::ParseError(e.to_string()))?;

        if let Some(errors) = linear_resp.errors {
            let msgs: Vec<String> = errors.into_iter().map(|e| e.message).collect();
            return Err(WorkItemError::ParseError(msgs.join("; ")));
        }

        let data = linear_resp
            .data
            .ok_or_else(|| WorkItemError::ParseError("missing data field".into()))?;

        nodes.extend(data.issues.nodes);
        match data.issues.page_info {
            Some(LinearPageInfo {
                has_next_page: true,
                end_cursor,
            }) => {
                after = Some(end_cursor.ok_or_else(|| {
                    WorkItemError::ParseError("pageInfo.hasNextPage without endCursor".into())
                })?);
            }
            _ => break,
        }
    }

    let mut items: Vec<WorkItem> = nodes
        .into_iter()
        .map(|i| WorkItem {
            id: i.identifier,
            title: i.title,
            url: i.url,
            // Linear dates can be full ISO-8601; take the date part.
            completed_at: i.completed_at.as_deref().and_then(date_prefix),
            labels: i.labels.nodes.into_iter().map(|l| l.name).collect(),
            milestone: i.project_milestone.map(|m| m.name),
            assignee: i.assignee.map(|a| a.name),
            hours: i.estimate.map(|points| points * hours_per_point),
        })
        .collect();

    // The server filter already requires completion; re-check so an item can
    // never be billed without a completion date inside the requested range.
    items.retain(|item| item.completed_at.is_some());
    retain_completed_since(&mut items, filter.since);

    // Client-side milestone filter for Linear (projectMilestone is fetched
    // but not filterable in all Linear plans).
    if let Some(ref milestone) = filter.milestone {
        items.retain(|item| {
            item.milestone
                .as_deref()
                .is_some_and(|m| m.eq_ignore_ascii_case(milestone))
        });
    }

    Ok(items)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_source_serde_roundtrip() {
        let src = WorkItemSource::GitHub {
            owner: "acme".into(),
            repo: "widgets".into(),
            token: "ghp_abc".into(),
        };
        let json = serde_json::to_string(&src).unwrap();
        assert!(json.contains(r#""type":"GitHub"#));
        let back: WorkItemSource = serde_json::from_str(&json).unwrap();
        match back {
            WorkItemSource::GitHub { owner, repo, .. } => {
                assert_eq!(owner, "acme");
                assert_eq!(repo, "widgets");
            }
            _ => panic!("expected GitHub variant"),
        }
    }

    #[test]
    fn linear_source_serde_roundtrip() {
        let src = WorkItemSource::Linear {
            api_key: "lin_key".into(),
            team_id: "TEAM1".into(),
            hours_per_point: 2.0,
        };
        let json = serde_json::to_string(&src).unwrap();
        assert!(json.contains(r#""type":"Linear"#));
        let back: WorkItemSource = serde_json::from_str(&json).unwrap();
        match back {
            WorkItemSource::Linear {
                api_key,
                team_id,
                hours_per_point,
            } => {
                assert_eq!(api_key, "lin_key");
                assert_eq!(team_id, "TEAM1");
                assert_eq!(hours_per_point, 2.0);
            }
            _ => panic!("expected Linear variant"),
        }
    }

    #[test]
    fn work_item_construction() {
        let item = WorkItem {
            id: "42".into(),
            title: "Fix login bug".into(),
            url: "https://github.com/acme/app/issues/42".into(),
            completed_at: Some(NaiveDate::from_ymd_opt(2026, 3, 1).unwrap()),
            labels: vec!["bug".into(), "auth".into()],
            milestone: Some("v1.0".into()),
            assignee: Some("alice".into()),
            hours: Some(2.5),
        };
        assert_eq!(item.id, "42");
        assert_eq!(item.labels.len(), 2);
        assert_eq!(item.hours, Some(2.5));
    }

    #[test]
    fn filter_defaults() {
        let f = WorkItemFilter::default();
        assert!(f.milestone.is_none());
        assert!(f.label.is_none());
        assert!(f.since.is_none());
        assert!(f.assignee.is_none());
    }

    #[test]
    fn estimate_with_hours() {
        let items = vec![WorkItem {
            id: "1".into(),
            title: "Implement feature".into(),
            url: "https://example.com/1".into(),
            completed_at: None,
            labels: vec![],
            milestone: None,
            assignee: None,
            hours: Some(3.0),
        }];
        let estimates = estimate_line_items(&items, 15000); // $150/hr
        assert_eq!(estimates.len(), 1);
        let e = &estimates[0];
        assert_eq!(e.hours, Decimal::new(3, 0));
        assert_eq!(e.rate_cents, 15000);
        assert_eq!(e.total_cents, 45000);
        assert_eq!(e.description, "Implement feature");
    }

    #[test]
    fn estimate_without_hours_defaults_to_one() {
        let items = vec![WorkItem {
            id: "2".into(),
            title: "Small fix".into(),
            url: "https://example.com/2".into(),
            completed_at: None,
            labels: vec![],
            milestone: None,
            assignee: None,
            hours: None,
        }];
        let estimates = estimate_line_items(&items, 10000);
        assert_eq!(estimates.len(), 1);
        let e = &estimates[0];
        assert_eq!(e.hours, Decimal::new(1, 0));
        assert_eq!(e.total_cents, 10000);
    }

    #[test]
    fn estimate_multiple_items() {
        let items = vec![
            WorkItem {
                id: "1".into(),
                title: "Task A".into(),
                url: "https://example.com/1".into(),
                completed_at: None,
                labels: vec![],
                milestone: None,
                assignee: None,
                hours: Some(2.0),
            },
            WorkItem {
                id: "2".into(),
                title: "Task B".into(),
                url: "https://example.com/2".into(),
                completed_at: None,
                labels: vec![],
                milestone: None,
                assignee: None,
                hours: Some(0.5),
            },
        ];
        let estimates = estimate_line_items(&items, 20000);
        assert_eq!(estimates.len(), 2);
        assert_eq!(estimates[0].total_cents, 40000);
        assert_eq!(estimates[1].total_cents, 10000);
    }

    #[test]
    fn estimate_empty_items() {
        let estimates = estimate_line_items(&[], 10000);
        assert!(estimates.is_empty());
    }

    #[test]
    fn estimate_fractional_hours() {
        let items = vec![WorkItem {
            id: "1".into(),
            title: "Quick task".into(),
            url: "https://example.com/1".into(),
            completed_at: None,
            labels: vec![],
            milestone: None,
            assignee: None,
            hours: Some(0.25),
        }];
        let estimates = estimate_line_items(&items, 10000);
        assert_eq!(estimates.len(), 1);
        assert_eq!(estimates[0].total_cents, 2500);
    }

    #[test]
    fn estimate_zero_rate() {
        let items = vec![WorkItem {
            id: "1".into(),
            title: "Pro bono".into(),
            url: "https://example.com/1".into(),
            completed_at: None,
            labels: vec![],
            milestone: None,
            assignee: None,
            hours: Some(5.0),
        }];
        let estimates = estimate_line_items(&items, 0);
        assert_eq!(estimates[0].total_cents, 0);
        assert_eq!(estimates[0].rate_cents, 0);
    }

    #[test]
    fn estimate_preserves_description() {
        let items = vec![WorkItem {
            id: "99".into(),
            title: "Implement OAuth2 PKCE flow".into(),
            url: "https://example.com/99".into(),
            completed_at: None,
            labels: vec![],
            milestone: None,
            assignee: None,
            hours: Some(8.0),
        }];
        let estimates = estimate_line_items(&items, 12500);
        assert_eq!(estimates[0].description, "Implement OAuth2 PKCE flow");
    }

    #[test]
    fn work_item_serde_roundtrip() {
        let item = WorkItem {
            id: "42".into(),
            title: "Add tests".into(),
            url: "https://github.com/acme/app/issues/42".into(),
            completed_at: Some(NaiveDate::from_ymd_opt(2026, 3, 10).unwrap()),
            labels: vec!["bug".into(), "priority".into()],
            milestone: Some("v1.0".into()),
            assignee: Some("alice".into()),
            hours: Some(3.5),
        };
        let json = serde_json::to_string(&item).unwrap();
        let back: WorkItem = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "42");
        assert_eq!(back.title, "Add tests");
        assert_eq!(
            back.completed_at,
            Some(NaiveDate::from_ymd_opt(2026, 3, 10).unwrap())
        );
        assert_eq!(back.labels, vec!["bug", "priority"]);
        assert_eq!(back.milestone.as_deref(), Some("v1.0"));
        assert_eq!(back.assignee.as_deref(), Some("alice"));
        assert_eq!(back.hours, Some(3.5));
    }

    #[test]
    fn work_item_no_optional_fields() {
        let item = WorkItem {
            id: "1".into(),
            title: "Bare".into(),
            url: "https://example.com".into(),
            completed_at: None,
            labels: vec![],
            milestone: None,
            assignee: None,
            hours: None,
        };
        let json = serde_json::to_string(&item).unwrap();
        let back: WorkItem = serde_json::from_str(&json).unwrap();
        assert!(back.completed_at.is_none());
        assert!(back.labels.is_empty());
        assert!(back.milestone.is_none());
        assert!(back.assignee.is_none());
        assert!(back.hours.is_none());
    }

    #[test]
    fn filter_serde_roundtrip() {
        let filter = WorkItemFilter {
            milestone: Some("v2.0".into()),
            label: Some("enhancement".into()),
            since: Some(NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()),
            assignee: Some("bob".into()),
        };
        let json = serde_json::to_string(&filter).unwrap();
        let back: WorkItemFilter = serde_json::from_str(&json).unwrap();
        assert_eq!(back.milestone.as_deref(), Some("v2.0"));
        assert_eq!(back.label.as_deref(), Some("enhancement"));
        assert_eq!(
            back.since,
            Some(NaiveDate::from_ymd_opt(2026, 1, 1).unwrap())
        );
        assert_eq!(back.assignee.as_deref(), Some("bob"));
    }

    #[test]
    fn filter_partial_fields() {
        let json = r#"{"milestone": "sprint-5"}"#;
        let filter: WorkItemFilter = serde_json::from_str(json).unwrap();
        assert_eq!(filter.milestone.as_deref(), Some("sprint-5"));
        assert!(filter.label.is_none());
        assert!(filter.since.is_none());
        assert!(filter.assignee.is_none());
    }

    #[test]
    fn invoice_line_estimate_serde() {
        let est = InvoiceLineEstimate {
            description: "Feature work".into(),
            hours: Decimal::new(5, 0),
            rate_cents: 15000,
            total_cents: 75000,
        };
        let json = serde_json::to_string(&est).unwrap();
        assert!(json.contains("Feature work"));
        assert!(json.contains("15000"));
        let back: InvoiceLineEstimate = serde_json::from_str(&json).unwrap();
        assert_eq!(back.description, "Feature work");
        assert_eq!(back.hours, Decimal::new(5, 0));
        assert_eq!(back.rate_cents, 15000);
        assert_eq!(back.total_cents, 75000);
    }

    #[test]
    fn work_item_error_display() {
        let err = WorkItemError::RequestFailed("timeout".into());
        assert_eq!(err.to_string(), "HTTP request failed: timeout");
        let err2 = WorkItemError::ParseError("bad json".into());
        assert_eq!(err2.to_string(), "failed to parse response: bad json");
    }

    #[test]
    fn github_issue_deserializes() {
        let json = r#"{
            "number": 123,
            "title": "Fix crash on startup",
            "html_url": "https://github.com/acme/app/issues/123",
            "closed_at": "2026-03-10T15:30:00Z",
            "labels": [{"name": "bug"}, {"name": "P1"}],
            "milestone": {"title": "v1.0"},
            "assignee": {"login": "alice"}
        }"#;
        let issue: GitHubIssue = serde_json::from_str(json).unwrap();
        assert_eq!(issue.number, 123);
        assert_eq!(issue.title, "Fix crash on startup");
        assert_eq!(issue.labels.len(), 2);
        assert_eq!(issue.labels[0].name, "bug");
        assert_eq!(issue.milestone.as_ref().unwrap().title, "v1.0");
        assert_eq!(issue.assignee.as_ref().unwrap().login, "alice");
    }

    #[test]
    fn github_issue_minimal() {
        let json = r#"{
            "number": 1,
            "title": "Test",
            "html_url": "https://github.com/a/b/issues/1",
            "closed_at": null,
            "labels": [],
            "milestone": null,
            "assignee": null
        }"#;
        let issue: GitHubIssue = serde_json::from_str(json).unwrap();
        assert_eq!(issue.number, 1);
        assert!(issue.closed_at.is_none());
        assert!(issue.labels.is_empty());
        assert!(issue.milestone.is_none());
        assert!(issue.assignee.is_none());
    }

    #[test]
    fn linear_response_deserializes_success() {
        let json = r#"{
            "data": {
                "issues": {
                    "nodes": [{
                        "identifier": "ENG-42",
                        "title": "Add dark mode",
                        "url": "https://linear.app/team/ENG-42",
                        "completedAt": "2026-03-05T10:00:00.000Z",
                        "labels": {"nodes": [{"name": "feature"}, {"name": "frontend"}]},
                        "assignee": {"name": "Bob Smith"},
                        "projectMilestone": {"name": "Q1 Release"},
                        "estimate": 3.0
                    }]
                }
            }
        }"#;
        let resp: LinearResponse = serde_json::from_str(json).unwrap();
        let data = resp.data.unwrap();
        assert_eq!(data.issues.nodes.len(), 1);
        let issue = &data.issues.nodes[0];
        assert_eq!(issue.identifier, "ENG-42");
        assert_eq!(issue.title, "Add dark mode");
        assert_eq!(issue.labels.nodes.len(), 2);
        assert_eq!(issue.assignee.as_ref().unwrap().name, "Bob Smith");
        assert_eq!(issue.project_milestone.as_ref().unwrap().name, "Q1 Release");
        assert_eq!(issue.estimate, Some(3.0));
    }

    #[test]
    fn linear_response_deserializes_error() {
        let json = r#"{
            "data": null,
            "errors": [{"message": "Authentication required"}]
        }"#;
        let resp: LinearResponse = serde_json::from_str(json).unwrap();
        assert!(resp.data.is_none());
        assert_eq!(resp.errors.as_ref().unwrap().len(), 1);
        assert_eq!(resp.errors.unwrap()[0].message, "Authentication required");
    }

    #[test]
    fn linear_issue_minimal() {
        let json = r#"{
            "identifier": "ENG-1",
            "title": "Minimal",
            "url": "https://linear.app/t/ENG-1",
            "completedAt": null,
            "labels": {"nodes": []},
            "assignee": null,
            "projectMilestone": null,
            "estimate": null
        }"#;
        let issue: LinearIssue = serde_json::from_str(json).unwrap();
        assert_eq!(issue.identifier, "ENG-1");
        assert!(issue.completed_at.is_none());
        assert!(issue.labels.nodes.is_empty());
        assert!(issue.assignee.is_none());
        assert!(issue.project_milestone.is_none());
        assert!(issue.estimate.is_none());
    }

    #[test]
    fn github_closed_at_date_parsing() {
        assert_eq!(
            date_prefix("2026-03-10T15:30:00Z"),
            Some(NaiveDate::from_ymd_opt(2026, 3, 10).unwrap())
        );
    }

    #[test]
    fn linear_completed_at_date_parsing() {
        // Full ISO-8601 from Linear
        assert_eq!(
            date_prefix("2026-03-05T10:00:00.000Z"),
            Some(NaiveDate::from_ymd_opt(2026, 3, 5).unwrap())
        );
    }

    #[test]
    fn date_prefix_rejects_short_or_non_ascii_without_panicking() {
        assert_eq!(date_prefix("2026-03"), None);
        assert_eq!(date_prefix("2026-03-0\u{e9}T00:00"), None);
        assert_eq!(date_prefix(""), None);
    }

    fn item_with_hours(hours: Option<f64>) -> WorkItem {
        WorkItem {
            id: "1".into(),
            title: "Task".into(),
            url: "https://example.com/1".into(),
            completed_at: None,
            labels: vec![],
            milestone: None,
            assignee: None,
            hours,
        }
    }

    #[test]
    fn estimate_float_hours_are_not_truncated() {
        // 0.3 h used to become 0.2999… h and 2999 cents.
        let e = &estimate_line_items(&[item_with_hours(Some(0.3))], 10000)[0];
        assert_eq!(e.hours, Decimal::new(3, 1));
        assert_eq!(e.hours.to_string(), "0.3");
        assert_eq!(e.total_cents, 3000);

        let e = &estimate_line_items(&[item_with_hours(Some(0.1 + 0.2))], 10000)[0];
        assert_eq!(e.total_cents, 3000);
    }

    #[test]
    fn estimate_total_rounds_half_up_to_cents() {
        // 1.5 h × 335¢ = 502.5¢ → 503¢ (not truncated, not banker's 502¢).
        let e = &estimate_line_items(&[item_with_hours(Some(1.5))], 335)[0];
        assert_eq!(e.total_cents, 503);
        // 1/3 h is kept to four decimals.
        let e = &estimate_line_items(&[item_with_hours(Some(1.0 / 3.0))], 9000)[0];
        assert_eq!(e.hours, Decimal::new(3333, 4));
        assert_eq!(e.total_cents, 3000);
    }

    fn issue_json(extra: &str) -> String {
        format!(
            r#"{{"number": 7, "title": "T", "html_url": "https://github.com/a/b/issues/7",
                "closed_at": "2026-03-10T15:30:00Z", "labels": [], "milestone": null,
                "assignee": null {extra}}}"#
        )
    }

    #[test]
    fn github_completed_excludes_not_planned_and_unmerged_prs() {
        let done: GitHubIssue =
            serde_json::from_str(&issue_json(r#", "state_reason": "completed""#)).unwrap();
        assert!(done.is_completed());

        let wontfix: GitHubIssue =
            serde_json::from_str(&issue_json(r#", "state_reason": "not_planned""#)).unwrap();
        assert!(!wontfix.is_completed());

        let merged: GitHubIssue = serde_json::from_str(&issue_json(
            r#", "pull_request": {"merged_at": "2026-03-10T15:30:00Z"}"#,
        ))
        .unwrap();
        assert!(merged.is_completed());

        let abandoned: GitHubIssue =
            serde_json::from_str(&issue_json(r#", "pull_request": {"merged_at": null}"#)).unwrap();
        assert!(!abandoned.is_completed());
    }

    #[test]
    fn github_issue_multiple_assignees_deserialize() {
        let issue: GitHubIssue = serde_json::from_str(&issue_json(
            r#", "assignees": [{"login": "alice"}, {"login": "bob"}]"#,
        ))
        .unwrap();
        assert_eq!(issue.assignees.len(), 2);
        assert_eq!(issue.assignees[1].login, "bob");
    }

    #[test]
    fn link_header_next_extraction() {
        let header = r#"<https://api.github.com/repositories/1/issues?page=2>; rel="next", <https://api.github.com/repositories/1/issues?page=5>; rel="last""#;
        assert_eq!(
            next_link(header).as_deref(),
            Some("https://api.github.com/repositories/1/issues?page=2")
        );
        let last_page = r#"<https://api.github.com/x?page=1>; rel="first", <https://api.github.com/x?page=4>; rel="prev""#;
        assert_eq!(next_link(last_page), None);
        assert_eq!(next_link(""), None);
    }

    fn milestones() -> Vec<GitHubMilestoneRef> {
        vec![
            GitHubMilestoneRef {
                number: 3,
                title: "v1.0".into(),
            },
            GitHubMilestoneRef {
                number: 9,
                title: "2026".into(),
            },
        ]
    }

    #[test]
    fn github_milestone_title_resolves_to_number() {
        assert_eq!(pick_github_milestone("v1.0", &milestones()).unwrap(), "3");
        assert_eq!(pick_github_milestone("V1.0 ", &milestones()).unwrap(), "3");
        // A numeric title wins over being read as a number.
        assert_eq!(pick_github_milestone("2026", &milestones()).unwrap(), "9");
        // A bare number that matches no title is taken as the number.
        assert_eq!(pick_github_milestone("12", &milestones()).unwrap(), "12");
        assert_eq!(pick_github_milestone("*", &[]).unwrap(), "*");
        assert_eq!(pick_github_milestone("None", &[]).unwrap(), "none");
        assert!(matches!(
            pick_github_milestone("v9", &milestones()),
            Err(WorkItemError::InvalidInput(_))
        ));
    }

    #[test]
    fn github_names_validated() {
        assert!(is_valid_github_name("acme-co"));
        assert!(is_valid_github_name("my.repo_2"));
        assert!(!is_valid_github_name(""));
        assert!(!is_valid_github_name(".."));
        assert!(!is_valid_github_name("a/b"));
        assert!(!is_valid_github_name("a?x=1"));
    }

    #[test]
    fn url_encode_escapes_reserved() {
        assert_eq!(url_encode("good first issue"), "good%20first%20issue");
        assert_eq!(url_encode("a&b=c"), "a%26b%3Dc");
    }

    #[test]
    fn since_filters_on_completion_date() {
        let mut items = vec![
            WorkItem {
                completed_at: Some(NaiveDate::from_ymd_opt(2026, 2, 28).unwrap()),
                ..item_with_hours(None)
            },
            WorkItem {
                completed_at: Some(NaiveDate::from_ymd_opt(2026, 3, 1).unwrap()),
                ..item_with_hours(None)
            },
            item_with_hours(None),
        ];
        retain_completed_since(&mut items, NaiveDate::from_ymd_opt(2026, 3, 1));
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].completed_at, NaiveDate::from_ymd_opt(2026, 3, 1));
    }

    #[test]
    fn linear_query_has_single_completed_at_and_pagination() {
        let filter = WorkItemFilter {
            since: NaiveDate::from_ymd_opt(2026, 1, 1),
            ..Default::default()
        };
        let q = linear_query("TEAM", &filter);
        assert_eq!(q.matches("completedAt:").count(), 1);
        assert!(q.contains("completedAt: { gte: \"2026-01-01T00:00:00.000Z\" }"));
        assert!(q.contains("after: $after"));
        assert!(q.contains("pageInfo"));

        let q = linear_query("TEAM", &WorkItemFilter::default());
        assert!(q.contains("completedAt: { null: false }"));
    }

    #[test]
    fn linear_query_escapes_values() {
        let filter = WorkItemFilter {
            label: Some("a\"b\nc\\".into()),
            ..Default::default()
        };
        let q = linear_query("T\"1", &filter);
        assert!(q.contains(r#"eq: "T\"1""#));
        assert!(q.contains(r#"eq: "a\"b\nc\\""#));
    }

    #[test]
    fn linear_page_info_deserializes() {
        let json = r#"{"data": {"issues": {"nodes": [],
            "pageInfo": {"hasNextPage": true, "endCursor": "abc"}}}}"#;
        let resp: LinearResponse = serde_json::from_str(json).unwrap();
        let info = resp.data.unwrap().issues.page_info.unwrap();
        assert!(info.has_next_page);
        assert_eq!(info.end_cursor.as_deref(), Some("abc"));
    }

    #[test]
    fn estimate_large_hourly_rate() {
        let items = vec![WorkItem {
            id: "1".into(),
            title: "Premium consulting".into(),
            url: "https://example.com/1".into(),
            completed_at: None,
            labels: vec![],
            milestone: None,
            assignee: None,
            hours: Some(100.0),
        }];
        let estimates = estimate_line_items(&items, 50000); // $500/hr
        assert_eq!(estimates[0].total_cents, 5_000_000);
    }

    #[test]
    fn linear_hours_per_point_defaults_to_one() {
        let src: WorkItemSource =
            serde_json::from_str(r#"{"type":"Linear","api_key":"k","team_id":"T"}"#).unwrap();
        assert!(matches!(
            src,
            WorkItemSource::Linear { hours_per_point, .. } if hours_per_point == 1.0
        ));
    }

    #[test]
    fn linear_rejects_non_positive_hours_per_point() {
        for bad in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            assert!(matches!(
                validate_hours_per_point(bad),
                Err(WorkItemError::InvalidInput(_))
            ));
        }
        assert!(validate_hours_per_point(0.5).is_ok());
    }
}
