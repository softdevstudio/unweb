# Case study sites

The made-up sites behind the case studies and use examples on [unweb.org](https://unweb.org/tags/case-studies/). Python 3, standard library only.

```
python3 server.py
```

Then open them in Unweb:

- `unweb http://127.0.0.1:8950/hotel`: a hotel that quotes a returning visitor more. Reload once and that stranger is a returning visitor; add one with Side by side and it isn't.
- `unweb http://127.0.0.1:8950/news`: a news site with three free articles. Reload until the paywall shows, then add a stranger beside it.
- `unweb http://127.0.0.1:8950/shop`: a shop that sets cookies before its banner is answered. Watch the strip's count.
- `unweb http://127.0.0.1:8950/travel`: prices by country. Give one stranger the location `http://127.0.0.1:8961` (stands in for Germany) and another `http://127.0.0.1:8962` (France).
- `unweb http://127.0.0.1:8950/store`: a headphone shop with a price for new visitors, one for returning visitors and one for each country. The picture on unweb.org's home page.
- `unweb http://127.0.0.1:8950/fare`: a fare that goes up each time the same stranger looks. Reload a few times, then add a stranger.
- `unweb http://127.0.0.1:8950/search`: a search page that moves a shop it has seen you visit to the top. Reload once, then add a stranger.
- `http://127.0.0.1:8950/ads`: a news page whose ad depends on the reader's country. Open it in a setup of strangers with the locations `http://127.0.0.1:8961`, `8962` (France) and `8963` (Spain), and one direct.
- `unweb http://127.0.0.1:8950/checkout`: a checkout A/B test. New visitors get A and B in turn; click Side by side to see both.

Type addresses with `http://`: the proxies don't handle HTTPS. The three proxies only handle plain `http://` addresses and tell the site the country with a header, where a real site would look at the IP address.
