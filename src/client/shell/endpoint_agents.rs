use super::*;

pub(super) fn render_collapsed(
    buffer: &mut Buffer,
    area: Rect,
    _endpoints: &[ClientShellEndpoint],
    _active_endpoint_id: &ClientEndpointId,
    config: &ClientShellConfig,
    hits: &mut ShellHitMap,
) {
    super::todo::render(buffer, area, config, &mut 0, hits, true);
}

pub(super) fn render_expanded(
    buffer: &mut Buffer,
    area: Rect,
    _agent_view_label: Option<&str>,
    _endpoints: &[ClientShellEndpoint],
    _active_endpoint_id: &ClientEndpointId,
    config: &ClientShellConfig,
    agent_scroll: &mut usize,
    hits: &mut ShellHitMap,
) {
    super::todo::render(buffer, area, config, agent_scroll, hits, false);
}

impl ClientShellState {
    // Agent navigation still focuses terminals, but must not scroll the Todo feed.
    pub(super) fn reveal_endpoint_agent(
        &mut self,
        _endpoint_id: &ClientEndpointId,
        _pane_id: &str,
        _body_height: u16,
    ) {
    }
}
