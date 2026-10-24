// The strip above each stranger: its language, its cookie count and its
// close button. It only talks to the app through its own commands, which
// change this stranger and nothing else.
(function () {
  'use strict';
  var invoke = window.__TAURI_INTERNALS__ && window.__TAURI_INTERNALS__.invoke;
  var lang = document.getElementById('lang');
  var cookies = document.getElementById('cookies');
  var note = document.getElementById('note');
  var close = document.getElementById('close');
  var filled = false;
  var current = '';

  function call(name, args) {
    note.textContent = '';
    if (!invoke) return Promise.resolve();
    return invoke(name, args || {}).catch(function (e) {
      note.textContent = String(e);
      lang.value = current;
    });
  }

  window.unwebStrip = function (s) {
    if (!filled) {
      s.langs.forEach(function (l) {
        var o = document.createElement('option');
        o.value = l[0];
        o.textContent = l[1];
        lang.appendChild(o);
      });
      filled = true;
    }
    current = s.lang || '';
    if (document.activeElement !== lang) lang.value = current;
    lang.disabled = !s.langPerStranger;
    if (!s.langPerStranger) lang.title = s.noLangReason;
    // "18 cookies from example.com, 3 from 2 other sites", with the count
    // for this site in bold.
    cookies.textContent = '';
    var text = String(s.cookies || '');
    var m = /^(\d+ cookies?)(.*)$/.exec(text);
    if (m) {
      var b = document.createElement('b');
      b.textContent = m[1];
      cookies.appendChild(b);
      cookies.appendChild(document.createTextNode(m[2]));
    } else {
      cookies.textContent = text;
    }
    close.hidden = !(s.tiles > 1);
  };

  lang.addEventListener('change', function () {
    if (lang.value === current) return;
    note.textContent = 'opening a new stranger…';
    call('set_language', { lang: lang.value });
  });
  close.addEventListener('click', function () { call('close_stranger'); });
})();
