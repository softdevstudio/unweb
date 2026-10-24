// The compare window: draws every stranger's screenshot side by side, each
// under a caption with its language, location, cookie count and address,
// on one canvas. Save image sends that canvas to the app as a PNG.
(function () {
  'use strict';
  var invoke = window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke;
  var canvas = document.getElementById('strip');
  var save = document.getElementById('save');
  var status = document.getElementById('status');
  var title = document.getElementById('title');

  var MARGIN = 16, GAP = 16, CAPTION = 78, MAX_H = 900;

  function load(src) {
    return new Promise(function (ok, fail) {
      var img = new Image();
      img.onload = function () { ok(img); };
      img.onerror = function () { fail(new Error('a screenshot could not be read')); };
      img.src = src;
    });
  }

  function fit(ctx, text, width) {
    text = String(text || '');
    if (ctx.measureText(text).width <= width) return text;
    while (text.length > 1 && ctx.measureText(text + '…').width > width) text = text.slice(0, -1);
    return text + '…';
  }

  window.unwebCompare = function (data) {
    var panels = data.panels || [];
    title.textContent = (panels.length === 1 ? '1 stranger' : panels.length + ' strangers') + ', ' + (data.taken || '');
    Promise.all(panels.map(function (p) { return load(p.image); })).then(function (imgs) {
      var h = Math.min(MAX_H, Math.min.apply(null, imgs.map(function (i) { return i.naturalHeight; })));
      var widths = imgs.map(function (i) { return Math.max(1, Math.round(i.naturalWidth * h / i.naturalHeight)); });
      var total = MARGIN * 2 + widths.reduce(function (a, b) { return a + b; }, 0) + GAP * (imgs.length - 1);
      canvas.width = total;
      canvas.height = MARGIN * 2 + CAPTION + h;
      var ctx = canvas.getContext('2d');
      ctx.fillStyle = '#ffffff';
      ctx.fillRect(0, 0, canvas.width, canvas.height);
      var x = MARGIN;
      imgs.forEach(function (img, i) {
        var p = panels[i], w = widths[i];
        ctx.fillStyle = '#111827';
        ctx.font = '600 17px system-ui, sans-serif';
        ctx.fillText(fit(ctx, p.title + ' · ' + p.lang + ' · ' + p.location, w), x, MARGIN + 18);
        ctx.fillStyle = '#374151';
        ctx.font = '14px system-ui, sans-serif';
        ctx.fillText(fit(ctx, p.cookies, w), x, MARGIN + 42);
        ctx.fillStyle = '#6b7280';
        ctx.fillText(fit(ctx, p.address, w), x, MARGIN + 64);
        ctx.drawImage(img, x, MARGIN + CAPTION, w, h);
        ctx.strokeStyle = '#d1d5db';
        ctx.strokeRect(x - 0.5, MARGIN + CAPTION - 0.5, w + 1, h + 1);
        x += w + GAP;
      });
      save.disabled = false;
      document.body.dataset.ready = 'yes';
    }, function (e) {
      status.textContent = String(e && e.message || e);
      status.className = 'error';
    });
  };

  save.addEventListener('click', function () {
    if (!invoke) return;
    save.disabled = true;
    status.className = '';
    status.textContent = 'saving…';
    invoke('save_compare', { image: canvas.toDataURL('image/png') }).then(function (path) {
      status.textContent = 'Saved to ' + path;
      document.body.dataset.saved = path;
      save.disabled = false;
    }, function (e) {
      status.textContent = String(e);
      status.className = 'error';
      save.disabled = false;
    });
  });
})();
