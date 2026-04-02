#!/bin/zsh
# Ralph Linear Source Adapter
# Implements the source interface for Linear issues and projects

LINEAR_ID=""
LINEAR_MODE=""  # "issue" or "project"
LINEAR_ISSUE_UUID=""
LINEAR_ISSUE_IDS=()  # for project mode: all issue UUIDs
LINEAR_TEAM_KEY=""
LINEAR_PROJECT_NAME=""

# GraphQL helper
linear_gql() {
  local query="$1"
  local variables="${2:-\{\}}"
  curl -s -X POST \
    -H "Content-Type: application/json" \
    -H "Authorization: $LINEAR_API_KEY" \
    --data "{\"query\": $(echo "$query" | jq -Rs .), \"variables\": $variables}" \
    https://api.linear.app/graphql
}

# Check for API errors in response
linear_check_error() {
  local response="$1"
  local errors
  errors=$(echo "$response" | jq -r '.errors[0].message // empty')
  if [ -n "$errors" ]; then
    echo "Error: Linear API: $errors"
    exit 1
  fi
}

# Detect if ID is an issue identifier or project UUID
detect_linear_type() {
  local id="$1"
  if [[ "$id" =~ ^[A-Z]+-[0-9]+$ ]]; then
    echo "issue"
  else
    echo "project"
  fi
}

# Fetch workflow state ID by name for a team
linear_find_state() {
  local team_id="$1"
  local state_name="$2"
  local response
  response=$(linear_gql "query { team(id: \"$team_id\") { states { nodes { id name } } } }")
  linear_check_error "$response"
  echo "$response" | jq -r ".data.team.states.nodes[] | select(.name == \"$state_name\") | .id"
}

# Update issue state
linear_set_issue_state() {
  local issue_id="$1"
  local state_name="$2"
  local team_id="$3"

  local state_id
  state_id=$(linear_find_state "$team_id" "$state_name")
  if [ -z "$state_id" ]; then
    echo "Warning: could not find '$state_name' state for team"
    return
  fi

  local response
  response=$(linear_gql "mutation { issueUpdate(id: \"$issue_id\", input: { stateId: \"$state_id\" }) { success } }")
  linear_check_error "$response"
}

source_init() {
  load_secret LINEAR_API_KEY "op://Remote Agents/Linear API Key/credential"

  if [ -z "$LINEAR_ID" ]; then
    read -r "LINEAR_ID?Project or issue ID: "
  fi

  if [ -z "$LINEAR_ID" ]; then
    echo "Error: Linear ID required"
    exit 1
  fi

  LINEAR_MODE=$(detect_linear_type "$LINEAR_ID")
  echo "Detected Linear $LINEAR_MODE: $LINEAR_ID"
}

source_fetch_prd() {
  if [ "$LINEAR_MODE" = "issue" ]; then
    _fetch_issue_prd
  else
    _fetch_project_prd
  fi
}

_fetch_issue_prd() {
  echo "Fetching PRD from Linear issue $LINEAR_ID..."
  local response
  response=$(linear_gql "query { issue(id: \"$LINEAR_ID\") { id title description team { id key } } }")
  linear_check_error "$response"

  LINEAR_ISSUE_UUID=$(echo "$response" | jq -r '.data.issue.id')
  LINEAR_TEAM_KEY=$(echo "$response" | jq -r '.data.issue.team.key')
  local team_id
  team_id=$(echo "$response" | jq -r '.data.issue.team.id')
  local title
  title=$(echo "$response" | jq -r '.data.issue.title')
  local description
  description=$(echo "$response" | jq -r '.data.issue.description // empty')

  if [ -z "$LINEAR_ISSUE_UUID" ] || [ "$LINEAR_ISSUE_UUID" = "null" ]; then
    echo "Error: could not fetch Linear issue $LINEAR_ID"
    exit 1
  fi

  if [ -z "$description" ]; then
    echo "Warning: issue has no description, using title as PRD"
    description="# $title"
  fi

  PRD="$description"
  SOURCE_LABEL="Linear issue $LINEAR_ID"
  LINEAR_ISSUE_IDS=("$LINEAR_ISSUE_UUID")
  LINEAR_TEAM_ID="$team_id"
}

_fetch_project_prd() {
  echo "Fetching PRD from Linear project $LINEAR_ID..."
  local response
  response=$(linear_gql "query { project(id: \"$LINEAR_ID\") { id name description issues(orderBy: { field: SORT_ORDER }) { nodes { id identifier title description } } } }")
  linear_check_error "$response"

  LINEAR_PROJECT_NAME=$(echo "$response" | jq -r '.data.project.name')
  local project_desc
  project_desc=$(echo "$response" | jq -r '.data.project.description // empty')

  if [ -z "$LINEAR_PROJECT_NAME" ] || [ "$LINEAR_PROJECT_NAME" = "null" ]; then
    echo "Error: could not fetch Linear project $LINEAR_ID"
    exit 1
  fi

  # Collect issue UUIDs for status updates
  LINEAR_ISSUE_IDS=($(echo "$response" | jq -r '.data.project.issues.nodes[].id'))

  # Get team info from first issue
  local first_issue_id="${LINEAR_ISSUE_IDS[1]}"
  if [ -n "$first_issue_id" ]; then
    local issue_resp
    issue_resp=$(linear_gql "query { issue(id: \"$first_issue_id\") { team { id key } } }")
    LINEAR_TEAM_ID=$(echo "$issue_resp" | jq -r '.data.issue.team.id')
    LINEAR_TEAM_KEY=$(echo "$issue_resp" | jq -r '.data.issue.team.key')
  fi

  local issue_count
  issue_count=$(echo "$response" | jq '.data.project.issues.nodes | length')

  if [ "$issue_count" -eq 0 ]; then
    echo "Error: project has no issues"
    exit 1
  fi

  echo "Found $issue_count issues in project '$LINEAR_PROJECT_NAME'"

  # Build combined PRD: project overview + each issue as a task
  PRD="# $LINEAR_PROJECT_NAME"
  if [ -n "$project_desc" ]; then
    PRD="$PRD

## Overview
$project_desc"
  fi

  PRD="$PRD

## Tasks"

  local issues_json
  issues_json=$(echo "$response" | jq -c '.data.project.issues.nodes[]')
  while IFS= read -r issue; do
    local ident title desc
    ident=$(echo "$issue" | jq -r '.identifier')
    title=$(echo "$issue" | jq -r '.title')
    desc=$(echo "$issue" | jq -r '.description // empty')

    PRD="$PRD

### $ident: $title"
    if [ -n "$desc" ]; then
      PRD="$PRD
$desc"
    fi
  done <<< "$issues_json"

  SOURCE_LABEL="Linear project '$LINEAR_PROJECT_NAME'"
}

source_comment() {
  local msg="$1"
  if [ "$LINEAR_MODE" = "issue" ]; then
    linear_gql "mutation { issueCommentCreate(input: { issueId: \"$LINEAR_ISSUE_UUID\", body: $(echo "$msg" | jq -Rs .) }) { success } }" >/dev/null
  else
    # Project mode: comment on all issues
    for uuid in "${LINEAR_ISSUE_IDS[@]}"; do
      linear_gql "mutation { issueCommentCreate(input: { issueId: \"$uuid\", body: $(echo "$msg" | jq -Rs .) }) { success } }" >/dev/null
    done
  fi
}

source_create_pr() {
  local branch="$1"
  git push -u origin "$branch"

  local body="## Summary
Automated implementation by Ralph.

Source: $SOURCE_LABEL"

  if [ "$LINEAR_MODE" = "issue" ]; then
    body="$body

Linear: $LINEAR_ID"
  else
    body="$body

Linear Project: $LINEAR_PROJECT_NAME"
    # List all issue identifiers
    for uuid in "${LINEAR_ISSUE_IDS[@]}"; do
      local ident
      ident=$(linear_gql "query { issue(id: \"$uuid\") { identifier } }" | jq -r '.data.issue.identifier')
      body="$body
- $ident"
    done
  fi

  body="$body

## Progress Log
\`\`\`
$(cat progress.txt)
\`\`\`"

  PR_URL=$(gh pr create --draft \
    --title "feat: $SOURCE_LABEL" \
    --body "$body")

  source_comment "Ralph completed all tasks!

PR: $PR_URL"
}

source_worktree_suffix() {
  if [ "$LINEAR_MODE" = "issue" ]; then
    echo "linear-$LINEAR_ID"
  else
    # Use sanitized project name or ID
    local slug
    slug=$(echo "$LINEAR_PROJECT_NAME" | tr '[:upper:]' '[:lower:]' | tr ' ' '-' | tr -cd 'a-z0-9-')
    echo "linear-${slug:-$LINEAR_ID}"
  fi
}

source_branch_name() {
  if [ "$LINEAR_MODE" = "issue" ]; then
    echo "ralph/$LINEAR_ID"
  else
    local slug
    slug=$(echo "$LINEAR_PROJECT_NAME" | tr '[:upper:]' '[:lower:]' | tr ' ' '-' | tr -cd 'a-z0-9-')
    echo "ralph/linear-${slug:-$LINEAR_ID}"
  fi
}

source_on_start() {
  [ -z "$LINEAR_TEAM_ID" ] && return
  for uuid in "${LINEAR_ISSUE_IDS[@]}"; do
    linear_set_issue_state "$uuid" "In Progress" "$LINEAR_TEAM_ID"
  done
}

source_on_complete() {
  [ -z "$LINEAR_TEAM_ID" ] && return
  for uuid in "${LINEAR_ISSUE_IDS[@]}"; do
    linear_set_issue_state "$uuid" "Done" "$LINEAR_TEAM_ID"
  done
}
