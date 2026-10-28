// Apply the saved theme before first paint to avoid a flash. Loaded as a classic (blocking) script.
try { var t = localStorage.getItem('ie-theme'); if (t === 'light' || t === 'dark') document.documentElement.dataset.theme = t; } catch (e) {}
