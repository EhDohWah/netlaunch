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
