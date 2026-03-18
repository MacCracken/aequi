use chrono::NaiveDate;
use rust_decimal::prelude::ToPrimitive;
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use thiserror::Error;

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
    },
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
    pub milestone: Option<String>,
    pub label: Option<String>,
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
struct GitHubAssignee {
    login: String,
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
        WorkItemSource::Linear { api_key, team_id } => {
            fetch_linear_issues(api_key, team_id, filter).await
        }
    }
}

/// Convert work items into invoice line estimates at the given hourly rate.
///
/// Items without an `hours` value default to 1 hour.
pub fn estimate_line_items(items: &[WorkItem], hourly_rate_cents: i64) -> Vec<InvoiceLineEstimate> {
    items
        .iter()
        .map(|item| {
            let hours_f = item.hours.unwrap_or(1.0);
            let hours = Decimal::from_f64_retain(hours_f).unwrap_or_else(|| Decimal::new(1, 0));
            let rate = Decimal::new(hourly_rate_cents, 0);
            let total = hours * rate;
            let total_cents = total.trunc().to_i64().unwrap_or(0);

            InvoiceLineEstimate {
                description: item.title.clone(),
                hours,
                rate_cents: hourly_rate_cents,
                total_cents,
            }
        })
        .collect()
}

// ---------------------------------------------------------------------------
// GitHub fetcher
// ---------------------------------------------------------------------------

async fn fetch_github_issues(
    owner: &str,
    repo: &str,
    token: &str,
    filter: &WorkItemFilter,
) -> Result<Vec<WorkItem>, WorkItemError> {
    let client = reqwest::Client::new();

    let mut url =
        format!("https://api.github.com/repos/{owner}/{repo}/issues?state=closed&per_page=100");

    // URL-encode filter values to prevent query parameter injection
    if let Some(ref milestone) = filter.milestone {
        let encoded: String = milestone
            .bytes()
            .map(|b| match b {
                b'0'..=b'9' | b'a'..=b'z' | b'A'..=b'Z' | b'-' | b'_' | b'.' => {
                    (b as char).to_string()
                }
                _ => format!("%{b:02X}"),
            })
            .collect();
        url.push_str(&format!("&milestone={encoded}"));
    }
    if let Some(ref label) = filter.label {
        let encoded: String = label
            .bytes()
            .map(|b| match b {
                b'0'..=b'9' | b'a'..=b'z' | b'A'..=b'Z' | b'-' | b'_' | b'.' => {
                    (b as char).to_string()
                }
                _ => format!("%{b:02X}"),
            })
            .collect();
        url.push_str(&format!("&labels={encoded}"));
    }
    if let Some(since) = filter.since {
        url.push_str(&format!("&since={since}T00:00:00Z"));
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

    let issues: Vec<GitHubIssue> = resp
        .json()
        .await
        .map_err(|e| WorkItemError::ParseError(e.to_string()))?;

    let mut items: Vec<WorkItem> = issues
        .into_iter()
        .map(|i| {
            let completed_at = i
                .closed_at
                .as_deref()
                .and_then(|s| s.get(..10))
                .and_then(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").ok());

            WorkItem {
                id: i.number.to_string(),
                title: i.title,
                url: i.html_url,
                completed_at,
                labels: i.labels.into_iter().map(|l| l.name).collect(),
                milestone: i.milestone.map(|m| m.title),
                assignee: i.assignee.map(|a| a.login),
                hours: None,
            }
        })
        .collect();

    // Client-side assignee filter (GitHub API doesn't support assignee on
    // the list-issues endpoint for arbitrary values).
    if let Some(ref assignee) = filter.assignee {
        items.retain(|item| {
            item.assignee
                .as_deref()
                .is_some_and(|a| a.eq_ignore_ascii_case(assignee))
        });
    }

    Ok(items)
}

// ---------------------------------------------------------------------------
// Linear fetcher
// ---------------------------------------------------------------------------

async fn fetch_linear_issues(
    api_key: &str,
    team_id: &str,
    filter: &WorkItemFilter,
) -> Result<Vec<WorkItem>, WorkItemError> {
    let client = reqwest::Client::new();

    // Escape GraphQL string values to prevent injection
    fn gql_escape(s: &str) -> String {
        s.replace('\\', "\\\\").replace('"', "\\\"")
    }

    let safe_team_id = gql_escape(team_id);
    let mut conditions = vec![
        format!("team: {{ id: {{ eq: \"{safe_team_id}\" }} }}"),
        "completedAt: { neq: null }".to_string(),
    ];

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
    if let Some(since) = filter.since {
        conditions.push(format!("completedAt: {{ gte: \"{since}T00:00:00.000Z\" }}"));
    }

    let filter_str = conditions.join(", ");

    let query = format!(
        r#"query {{
  issues(filter: {{ {filter_str} }}, first: 100) {{
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
  }}
}}"#
    );

    let body = serde_json::json!({ "query": query });

    let resp = client
        .post("https://api.linear.app/graphql")
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

    let mut items: Vec<WorkItem> = data
        .issues
        .nodes
        .into_iter()
        .map(|i| {
            let completed_at = i.completed_at.as_deref().and_then(|s| {
                // Linear dates can be full ISO-8601; take first 10 chars.
                let date_part = if s.len() >= 10 { &s[..10] } else { s };
                NaiveDate::parse_from_str(date_part, "%Y-%m-%d").ok()
            });

            WorkItem {
                id: i.identifier,
                title: i.title,
                url: i.url,
                completed_at,
                labels: i.labels.nodes.into_iter().map(|l| l.name).collect(),
                milestone: i.project_milestone.map(|m| m.name),
                assignee: i.assignee.map(|a| a.name),
                hours: i.estimate,
            }
        })
        .collect();

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
        };
        let json = serde_json::to_string(&src).unwrap();
        assert!(json.contains(r#""type":"Linear"#));
        let back: WorkItemSource = serde_json::from_str(&json).unwrap();
        match back {
            WorkItemSource::Linear { api_key, team_id } => {
                assert_eq!(api_key, "lin_key");
                assert_eq!(team_id, "TEAM1");
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
        // Verify the date parsing logic used in the GitHub fetcher
        let closed_at = "2026-03-10T15:30:00Z";
        let date_part = &closed_at[..10];
        let parsed = NaiveDate::parse_from_str(date_part, "%Y-%m-%d").unwrap();
        assert_eq!(parsed, NaiveDate::from_ymd_opt(2026, 3, 10).unwrap());
    }

    #[test]
    fn linear_completed_at_date_parsing() {
        // Full ISO-8601 from Linear
        let completed_at = "2026-03-05T10:00:00.000Z";
        let date_part = &completed_at[..10];
        let parsed = NaiveDate::parse_from_str(date_part, "%Y-%m-%d").unwrap();
        assert_eq!(parsed, NaiveDate::from_ymd_opt(2026, 3, 5).unwrap());
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
}
