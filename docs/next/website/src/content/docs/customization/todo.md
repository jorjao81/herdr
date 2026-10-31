---
title: Custom Todo feed
description: Publish personal updates and click actions in the fork's Todo panel.
---

This fork replaces the desktop and mobile switcher agents lists with Todo. Machines, workspaces,
terminals, and agent detection keep their existing behavior. Agent keyboard
shortcuts still navigate terminals.

The Todo entries API is a local JSON file. Calendar, GitLab, Teams, and other
providers can run independently in any language and publish a combined feed.
No accounts or demo notifications are connected by default.

Set `HERDR_TODO_FILE=/absolute/path/todo.json` before starting the client.
Otherwise the file is `todo.json` in Herdr's configuration directory
(`XDG_CONFIG_HOME/herdr` when that variable is set). Feed changes and time
windows are checked once per second, including while the terminal is idle.
A missing file produces “Nothing pending”; malformed data produces an error
and clears the previous feed. Details are written to the Herdr log.

```json
{
  "entries": [
    {
      "source": "calendar",
      "id": "meeting-2026-11-02",
      "kind": "meeting",
      "title": "Daily standup in 5 minutes",
      "detail": "Engineering",
      "priority": 100,
      "visible_from": 1793609700,
      "expires_at": 1793610900,
      "action": ["open", "https://teams.microsoft.com/"]
    },
    {
      "source": "gitlab",
      "id": "project-42-mr-17",
      "kind": "review_requested",
      "title": "Review: improve search",
      "detail": "Assigned to me",
      "priority": 20,
      "action": ["open", "https://gitlab.com/example/project/-/merge_requests/17"]
    },
    {
      "source": "teams",
      "id": "unread",
      "title": "3 unread messages",
      "action": ["/absolute/path/to/my-teams-action", "unread"]
    }
  ]
}
```

Each entry requires nonempty `source`, `id`, and `title`. `(source, id)` must
be unique. `detail` defaults to empty, `priority` to zero, `action` to an empty
array. Higher priorities appear first; ties preserve feed order. Optional
`visible_from` (inclusive) and `expires_at` (exclusive) are Unix seconds in UTC.
For a near meeting, publish visibility from e.g. ten minutes before its start
until its end. Titles are supplied by providers; the client does not compute
countdown text. Mouse wheel scrolls the expanded list.

Entries render as compact cards: a colored Nerd Font icon and title, a status
line, and up to two lines of the latest comment. Long titles and previews end
with an ellipsis; Unicode display widths are respected. The whole visible card,
including its comment, triggers the entry's action. Mouse wheel scrolling moves
through card lines. Collapsing the sidebar keeps each entry's state icon.
The mobile switcher uses the same cards and comment previews.

Optional `kind` selects the built-in icon, color, and status:

| `kind` | Nerd Font Octicon | Meaning |
| --- | --- | --- |
| `meeting` | calendar | Upcoming meeting |
| `merge_request` | git-pull-request | Open MR |
| `review_requested` | code-review | Someone requested your review |
| `assigned_comments` | comment-discussion | Your MR has new comments or unresolved threads |
| `assigned_approved` | check-circle | Your MR is approved |
| `comment_reply` | reply | Someone replied to your comment on another MR |
| `changes_requested` | comment | Changes requested |
| `merged` | git-merge | MR merged |
| `ci_failed` | x-circle | Pipeline failed |
| `message` | comment | Unread message |

Missing or unknown kinds use a neutral bell icon. An optional `icon` string
can override the glyph for custom providers. Configure your terminal with a
Nerd Font to display the built-in icons. Status text also distinguishes events.
Glyphs come from the [Nerd Fonts Octicons map](https://github.com/ryanoasis/nerd-fonts/blob/master/glyphnames.json).

Providers decide which relationship/event applies and include the comment text;
the client does not infer it from GitLab or fetch thread contents. For example:

```json
{
  "source": "gitlab",
  "id": "api-mr-142-thread-9",
  "kind": "assigned_comments",
  "title": "!142 Fix login retry",
  "detail": "api · 2 unresolved threads",
  "comment": {
    "author": "Avery",
    "text": "Could we retry the request before failing? This needs a regression test."
  },
  "action": ["open", "https://gitlab.com/example/api/-/merge_requests/142#note_9"]
}
```

`comment.author` is optional; `comment.text` contains the actual comment body.
Whitespace is collapsed for the preview. Use a thread-specific ID and action URL
to show multiple conversations on one MR and open the exact discussion.

Clicking an entry runs `action[0]` with the remaining literal arguments, without
a shell. It runs locally on the client, even when viewing remote machines,
with detached terminal I/O; failures go to the log. Empty actions are inert.
Use an executable script for custom workflows. On macOS `open` handles URLs;
other systems should use a suitable local launcher or helper.

Treat the feed as trusted local configuration because it can run commands.
Keep credentials in your providers, outside the feed. The client neither
fetches third-party data nor performs OAuth. Providers should authenticate
with their respective services, filter assigned/open MRs and unread messages,
and omit completed items. Publish via a temporary file and atomic rename;
coordinate multiple providers through one aggregator to avoid lost updates.
The feed is limited to 1 MiB. Restart the client to change its feed path.

Try the example by saving the JSON to a local file and launching
`HERDR_TODO_FILE=/absolute/path/todo.json herdr`. The sample calendar item only
appears within its example timestamps; adjust these for your meeting.
