// Words for a window's state (SessionView.state from the Rust side), used
// by the site list and the Open windows panel.

export const STATE_LABELS = {
  launching: 'Starting the browser…',
  loading: 'Opening the login page…',
  filling: 'Filling in the login form…',
  waiting: 'Waiting for the site to answer…',
  waiting_for_2fa: 'Waiting for 2FA',
  logged_in: 'Logged in',
  handed_over: 'Yours to finish',
  failed: 'Failed',
  stopped: 'Wrong site',
};

/** True while the login itself runs (Close, Release and Retry wait). */
export const isBusy = (state) => ['launching', 'loading', 'filling', 'waiting'].includes(state);

/** The colour class: busy, ok, warn or error. */
export function stateKind(state) {
  if (isBusy(state)) return 'busy';
  if (state === 'logged_in') return 'ok';
  if (state === 'waiting_for_2fa' || state === 'handed_over') return 'warn';
  return 'error';
}

// Short labels for the result of the last Open.
const OUTCOMES = {
  success: 'Logged in',
  mfa_handoff: 'Waiting for 2FA',
  failed: 'Failed',
  timeout: 'Timed out',
  origin_mismatch: 'Wrong site',
};

/**
 * What the list shows for a site: its open window's state if it has one,
 * else the result of its last Open, else null.
 * `{ kind, label, message, warnings, fix }`; kind is busy | ok | warn | error.
 */
export function siteStatus(site, sessions, logins) {
  const view = sessions[site.id];
  if (view) {
    const label = STATE_LABELS[view.state] ?? view.state;
    // "Logged in." would only repeat the label.
    const message = view.message === 'Logged in.' ? '' : view.message;
    return { kind: stateKind(view.state), label, message, warnings: view.warnings ?? [], fix: null };
  }
  const login = logins[site.id];
  if (!login) return null;
  if (login.busy) return { kind: 'busy', label: 'Starting…', message: '', warnings: [], fix: null };
  const kind = login.outcome === 'success' ? 'ok' : login.outcome === 'mfa_handoff' || login.outcome === 'timeout' ? 'warn' : 'error';
  return {
    kind,
    label: OUTCOMES[login.outcome] ?? login.outcome,
    message: login.outcome === 'success' ? '' : login.message,
    warnings: login.warnings ?? [],
    fix: login.fix ?? null,
  };
}

/** "Today 16:38", or the date for older times; "Never" when unused. */
export function lastUsed(site) {
  if (!site.last_used_at) return 'Never';
  const when = new Date(site.last_used_at);
  const time = when.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
  return when.toDateString() === new Date().toDateString() ? `Today ${time}` : `${when.toLocaleDateString()} ${time}`;
}
