#!/usr/bin/env python3
import sys, os, subprocess, pathlib
D = pathlib.Path(__file__).parent
CHROME = "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"
app = (D/"_app.html").read_text()
screen = (D/"_screen.html").read_text()

# pages: (file, width, height)
pages = [
    ("01-hero.html", 1600, 900),
    ("02-deliberation.html", 1600, 900),
    ("03-privacy.html", 1600, 900),
    ("04-providers.html", 1600, 900),
    ("05-grounded.html", 1600, 900),
    ("header.html", 1500, 500),
    ("_test-screen.html", 1440, 900),
    ("es-01-hero.html", 1600, 900),
    ("es-02-captura.html", 1600, 900),
    ("es-03-deliberacion.html", 1600, 900),
    ("es-04-privacidad.html", 1600, 900),
    ("es-05-motores.html", 1600, 900),
    ("es-06-multidispositivo.html", 1600, 900),
    ("es-07-cuadrado.html", 1080, 1080),
    ("es-08-contraste.html", 1600, 900),
    ("es-09-cita.html", 1600, 900),
    ("es-header.html", 1500, 500),
]
only = sys.argv[1:] if len(sys.argv) > 1 else None
for f, w, h in pages:
    if only and f not in only: continue
    src = D/f
    if not src.exists():
        print("skip (missing)", f); continue
    html = src.read_text().replace("__APP__", app).replace("__SCREEN__", screen)
    built = D/("_built_"+f)
    built.write_text(html)
    out = D/(f.replace(".html",".png"))
    subprocess.run([CHROME,"--headless=new","--disable-gpu","--no-sandbox",
        "--hide-scrollbars","--force-device-scale-factor=2",
        f"--window-size={w},{h}",f"--screenshot={out}",f"file://{built}"],
        check=True, capture_output=True)
    print("ok", out.name, f"{w}x{h} @2x")
