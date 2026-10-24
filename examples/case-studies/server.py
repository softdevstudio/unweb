#!/usr/bin/env python3
"""Made-up sites for Unweb's case studies and use examples (Python 3, stdlib only).

    python3 server.py

Serves, on 127.0.0.1:8950:
  /hotel   a hotel that quotes returning visitors a higher price
  /news    a news site with a meter of three free articles
  /shop    a shop that sets cookies before its banner is answered
  /travel  a travel page that shows a price for the visitor's country

and two stand-in proxies, 127.0.0.1:8961 and 127.0.0.1:8962, which tag each
request as coming from Germany or France. A real site would tell that from
the IP address; here the proxies say it out loud. Give a stranger one of them
as its location: http://127.0.0.1:8961
"""
import http.server, socket, threading

SITE = 8950
PROXIES = {8961: "DE", 8962: "FR"}

STYLE = """<style>body{margin:0;font:16px Georgia,serif;color:#1f2933;background:#f7f5f1}
header{padding:16px 26px;font-size:23px;color:#fff}main{padding:22px 26px}
.card{background:#fff;border:1px solid #e2ddd3;border-radius:10px;padding:18px;max-width:440px}
.tag{display:inline-block;padding:3px 9px;border-radius:5px;font:13px sans-serif;color:#fff}
.price{font-size:30px;margin:10px 0 4px}.small{font:14px sans-serif;color:#52606d}
.wall{background:#fff;border:2px solid #1f2933;border-radius:10px;padding:18px;max-width:440px;font:15px sans-serif}
.banner{position:fixed;left:0;right:0;bottom:0;background:#fff;border-top:2px solid #333;padding:12px 26px;font:14px sans-serif}
</style>"""

def page(title, color, body):
    return f"<!doctype html><html lang='en'><head><meta charset='utf-8'><title>{title}</title>{STYLE}</head><body><header style='background:{color}'>{title}</header><main>{body}</main></body></html>"

def cookies(handler):
    out = {}
    for part in (handler.headers.get("Cookie") or "").split(";"):
        if "=" in part:
            k, v = part.strip().split("=", 1)
            out[k] = v
    return out

class Site(http.server.BaseHTTPRequestHandler):
    def log_message(self, *a):
        pass

    def send(self, html, set_cookies=()):
        self.send_response(200)
        self.send_header("Content-Type", "text/html; charset=utf-8")
        for c in set_cookies:
            self.send_header("Set-Cookie", c + "; Path=/; Max-Age=86400")
        self.end_headers()
        self.wfile.write(html.encode())

    def do_GET(self):
        c = cookies(self)
        path = self.path.split("?")[0]
        if path == "/hotel":
            if "visited" in c:
                body = "<div class='card'><span class='tag' style='background:#b42318'>Only 2 rooms left at this price</span><h2>Harbour View Room, 2 nights</h2><div class='price'>€296</div><div class='small'>€148 a night, breakfast included</div></div>"
            else:
                body = "<div class='card'><span class='tag' style='background:#0f766e'>First stay with us? 10% off</span><h2>Harbour View Room, 2 nights</h2><div class='price'>€258</div><div class='small'>€129 a night, breakfast included</div></div>"
            self.send(page("Seaview Lodge", "#0e3a52", body), ["visited=1"])
        elif path == "/news":
            n = int(c.get("read", "0")) + 1
            if n > 3:
                body = "<div class='wall'><h2 style='margin-top:0'>You've read your 3 free articles</h2><p>Subscribe for €4.99 a month to keep reading.</p></div>"
            else:
                body = f"<div class='card'><span class='tag' style='background:#374151'>Free article {n} of 3</span><h2>City council approves new tram line</h2><p class='small'>The line will link the harbour with the university by 2029.</p></div>"
            self.send(page("The Daily Ledger", "#111827", body), [f"read={n}"])
        elif path == "/shop":
            body = "<div class='card'><h2>Trail Runner 3, size 42</h2><div class='price'>€89</div></div><img src='/pixel.gif' width='1' height='1' alt=''>" \
                   "<div class='banner'><b>Can we use cookies?</b> We'd like to remember your basket and measure our ads. <button>Accept</button> <button>Reject</button></div>" \
                   "<script>document.cookie='basket=0;path=/;max-age=86400';document.cookie='ab_test=b;path=/;max-age=86400';document.cookie='_ga=GA1.1.53;path=/;max-age=86400'</script>"
            self.send(page("Peak Outfitters", "#3f6212", body),
                      ["session=a81f; HttpOnly", "currency=EUR", "_fbp=fb.1.553", "_gcl_au=1.1.903"])
        elif path == "/travel":
            country = self.headers.get("X-Demo-Country", "")
            fares = {"DE": ("Germany", "€412", "Flights from Berlin"), "FR": ("France", "€389", "Flights from Paris")}
            name, fare, line = fares.get(country, ("your country", "€455", "Flights from London"))
            body = f"<div class='card'><span class='tag' style='background:#7c3aed'>Prices for {name}</span><h2>Lisbon, 4 nights in May</h2><div class='price'>{fare}</div><div class='small'>{line}, hotel included</div></div>"
            self.send(page("Wanderfare", "#4c1d95", body))
        elif path == "/pixel.gif":
            self.send_response(200)
            self.send_header("Content-Type", "image/gif")
            self.end_headers()
            self.wfile.write(b"GIF89a\x01\x00\x01\x00\x80\x00\x00\x00\x00\x00\xff\xff\xff!\xf9\x04\x01\x00\x00\x00\x00,\x00\x00\x00\x00\x01\x00\x01\x00\x00\x02\x02D\x01\x00;")
        else:
            self.send(page("Made-up sites", "#334155", "<p><a href='/hotel'>hotel</a> · <a href='/news'>news</a> · <a href='/shop'>shop</a> · <a href='/travel'>travel</a></p>"))

def proxy(port, country):
    """A stand-in HTTP proxy for plain http:// requests that adds X-Demo-Country."""
    srv = socket.socket(); srv.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
    srv.bind(("127.0.0.1", port)); srv.listen(50)
    def handle(c):
        try:
            data = b""
            while b"\r\n\r\n" not in data:
                chunk = c.recv(4096)
                if not chunk:
                    return
                data += chunk
            head, rest = data.split(b"\r\n\r\n", 1)
            lines = head.decode("latin-1").split("\r\n")
            method, target, _ = lines[0].split(" ", 2)
            if not target.startswith("http://"):
                c.sendall(b"HTTP/1.1 501 Not Implemented\r\nContent-Length: 0\r\n\r\n"); return
            hostport, _, path = target[7:].partition("/")
            host, _, p = hostport.partition(":")
            up = socket.create_connection((host, int(p or 80)))
            keep = [l for l in lines[1:] if not l.lower().startswith(("proxy-connection:", "connection:"))]
            up.sendall((f"{method} /{path} HTTP/1.1\r\n" + "\r\n".join(keep) + f"\r\nX-Demo-Country: {country}\r\nConnection: close\r\n\r\n").encode("latin-1") + rest)
            while True:
                chunk = up.recv(65536)
                if not chunk:
                    break
                c.sendall(chunk)
            up.close()
        finally:
            c.close()
    while True:
        c, _ = srv.accept()
        threading.Thread(target=handle, args=(c,), daemon=True).start()

if __name__ == "__main__":
    for port, country in PROXIES.items():
        threading.Thread(target=proxy, args=(port, country), daemon=True).start()
    print(f"made-up sites on http://127.0.0.1:{SITE}/, proxies on 127.0.0.1:8961 (DE) and 127.0.0.1:8962 (FR)")
    http.server.ThreadingHTTPServer(("127.0.0.1", SITE), Site).serve_forever()
