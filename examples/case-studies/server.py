#!/usr/bin/env python3
"""Made-up sites for Unweb's case studies and use examples (Python 3, stdlib only).

    python3 server.py

Serves, on 127.0.0.1:8950:
  /hotel   a hotel that quotes returning visitors a higher price
  /news    a news site with a meter of three free articles
  /shop    a shop that sets cookies before its banner is answered
  /travel  a travel page that shows a price for the visitor's country
  /store   a headphone shop: one price for new visitors, another for returning
           ones, and another again for each country
  /search  a search page that moves a shop it has seen you visit to the top
  /fare    an airline fare that rises each time the same visitor looks
  /ads     a news article whose ad depends on the reader's country
  /checkout a checkout page running an A/B test (visitors take turns, A then B)

and three stand-in proxies, 127.0.0.1:8961, 8962 and 8963, which tag each
request as coming from Germany, France or Spain. A real site would tell that from
the IP address; here the proxies say it out loud. Give a stranger one of them
as its location: http://127.0.0.1:8961
"""
import http.server, socket, threading

SITE = 8950
PROXIES = {8961: "DE", 8962: "FR", 8963: "ES"}
AB_TURN = [0]
AB_LOCK = threading.Lock()

STYLE = """<style>body{margin:0;font:16px Georgia,serif;color:#1f2933;background:#f7f5f1}
header{padding:16px 26px;font-size:23px;color:#fff}main{padding:22px 26px}
.card{background:#fff;border:1px solid #e2ddd3;border-radius:10px;padding:18px;max-width:440px}
.tag{display:inline-block;padding:3px 9px;border-radius:5px;font:13px sans-serif;color:#fff}
.price{font-size:30px;margin:10px 0 4px}.small{font:14px sans-serif;color:#52606d}
.wall{background:#fff;border:2px solid #1f2933;border-radius:10px;padding:18px;max-width:440px;font:15px sans-serif}
.row{display:flex;gap:18px;align-items:center}.art{flex:none}
.res{margin:0 0 16px;max-width:520px}.res a{font-size:18px;color:#1a4fa0;text-decoration:none}.res .u{font:13px sans-serif;color:#2f6f3e}
.ad{border:1px dashed #9aa5b1;border-radius:8px;padding:12px 14px;max-width:440px;margin-top:16px;font:15px sans-serif;background:#fff}
.ad .lbl{font:11px sans-serif;color:#7b8794;text-transform:uppercase;letter-spacing:.05em}
.btn{display:inline-block;margin-top:12px;padding:10px 18px;border-radius:8px;color:#fff;font:15px sans-serif}
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
        elif path == "/store":
            country = self.headers.get("X-Demo-Country", "")
            phones = "<svg class='art' width='92' height='92' viewBox='0 0 92 92'><circle cx='46' cy='46' r='44' fill='#e6eef7'/><path d='M22 52 a24 24 0 0 1 48 0' fill='none' stroke='#1f2933' stroke-width='6'/><rect x='14' y='48' width='16' height='24' rx='6' fill='#c2410c'/><rect x='62' y='48' width='16' height='24' rx='6' fill='#c2410c'/></svg>"
            if country == "DE":
                tag, color, price, note = "Preis für Deutschland", "#1d4ed8", "€219", "Versand in 2 Tagen"
            elif country == "FR":
                tag, color, price, note = "Prix pour la France", "#7c3aed", "€209", "Livraison en 3 jours"
            elif "seen" in c:
                tag, color, price, note = "Back again? Only 3 left", "#b42318", "£199", "Price for returning visitors"
            else:
                tag, color, price, note = "Welcome! 10% off your first order", "#0f766e", "£179", "Price for new visitors"
            body = f"<div class='card'><span class='tag' style='background:{color}'>{tag}</span><div class='row' style='margin-top:12px'>{phones}<div><h2 style='margin:0'>Northline Studio headphones</h2><div class='price'>{price}</div><div class='small'>{note}</div></div></div></div>"
            self.send(page("Northline Audio", "#1f2933", body), ["seen=1"])
        elif path == "/search":
            results = [("Best running shoes of the year, tested", "reviews.example/running-shoes"),
                       ("Trail Runner 3, size 42, from €89", "peak-outfitters.example/trail-runner-3"),
                       ("How to choose running shoes for flat feet", "runnersclinic.example/guide"),
                       ("Running shoes, 40% off this week", "stridemart.example/sale")]
            if "hist" in c:
                results = [results[1], results[3], results[0], results[2]]
                top = "<p class='small'>Showing results based on your recent visits</p>"
            else:
                top = "<p class='small'>About 41,200 results</p>"
            items = "".join(f"<div class='res'><a href='#'>{t}</a><div class='u'>{u}</div></div>" for t, u in results)
            body = f"<div style='font:16px sans-serif;border:1px solid #cbd2d9;border-radius:22px;padding:8px 16px;max-width:480px;background:#fff'>running shoes</div>{top}{items}"
            self.send(page("Findly", "#1a4fa0", body), ["hist=peak-outfitters"])
        elif path == "/fare":
            n = int(c.get("looks", "0")) + 1
            fares = ["€189", "€214", "€239", "€264"]
            fare = fares[min(n, 4) - 1]
            warn = "<span class='tag' style='background:#b42318'>Fares are rising. 4 people are looking at this flight</span>" if n > 2 else "<span class='tag' style='background:#0f766e'>Good fare for this route</span>"
            body = f"<div class='card'>{warn}<h2>Lisbon to Rome, Friday 14 May</h2><div class='small'>Direct, 07:40 to 11:25, cabin bag included</div><div class='price'>{fare}</div><div class='small'>One adult, one way</div></div>"
            self.send(page("SkyBridge Air", "#0b4f6c", body), [f"looks={n}"])
        elif path == "/ads":
            country = self.headers.get("X-Demo-Country", "")
            ads = {"DE": ("Kessler Versicherung", "Autoversicherung ab 9 € im Monat", "#1d4ed8"),
                   "FR": ("Maison Duval", "Soldes d'automne : -30 % sur les manteaux", "#7c3aed"),
                   "ES": ("Sol y Mar Viajes", "Escapadas de fin de semana desde 99 €", "#c2410c"),
                   "": ("Thornbury Bank", "Switch your current account and get £150", "#0f766e")}
            brand, line, color = ads.get(country, ads[""])
            body = f"<h2 style='margin-top:0'>Harbour tram line opens two years early</h2><p class='small'>The new line links the harbour with the university. Trams run every six minutes from Monday.</p><div class='ad'><div class='lbl'>Advertisement</div><b style='color:{color}'>{brand}</b><div>{line}</div></div>"
            self.send(page("The Morning Post", "#111827", body))
        elif path == "/checkout":
            v = c.get("ab")
            if v not in ("A", "B"):
                with AB_LOCK:
                    v = "AB"[AB_TURN[0] % 2]
                    AB_TURN[0] += 1
            if v == "A":
                inner = "<div class='price'>€89</div><div class='small'>Free delivery in 3 to 5 days</div><span class='btn' style='background:#15803d'>Buy now</span>"
            else:
                inner = "<div class='price'>€84 <span class='small' style='text-decoration:line-through'>€89</span></div><div class='small'>Free returns for 60 days. Delivery €5</div><span class='btn' style='background:#c2410c'>Add to basket</span>"
            body = f"<div class='card'><span class='tag' style='background:#374151'>Version {v}</span><h2>Trail Runner 3, size 42</h2>{inner}</div>"
            self.send(page("Peak Outfitters", "#3f6212", body), [f"ab={v}"])
        elif path == "/pixel.gif":
            self.send_response(200)
            self.send_header("Content-Type", "image/gif")
            self.end_headers()
            self.wfile.write(b"GIF89a\x01\x00\x01\x00\x80\x00\x00\x00\x00\x00\xff\xff\xff!\xf9\x04\x01\x00\x00\x00\x00,\x00\x00\x00\x00\x01\x00\x01\x00\x00\x02\x02D\x01\x00;")
        else:
            self.send(page("Made-up sites", "#334155", "<p><a href='/hotel'>hotel</a> · <a href='/news'>news</a> · <a href='/shop'>shop</a> · <a href='/travel'>travel</a> · <a href='/store'>store</a> · <a href='/search'>search</a> · <a href='/fare'>fare</a> · <a href='/ads'>ads</a> · <a href='/checkout'>checkout</a></p>"))

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
    print(f"made-up sites on http://127.0.0.1:{SITE}/, proxies on 127.0.0.1:8961 (DE), 8962 (FR) and 8963 (ES)")
    http.server.ThreadingHTTPServer(("127.0.0.1", SITE), Site).serve_forever()
