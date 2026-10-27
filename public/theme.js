// Runs in <head> before paint: applies an explicitly chosen theme, if any.
(function () {
  try {
    var theme = localStorage.getItem('specsentry-theme');
    if (theme === 'light' || theme === 'dark') document.documentElement.dataset.theme = theme;
  } catch (_) {
    // Storage may be blocked; the system preference still applies.
  }
})();
