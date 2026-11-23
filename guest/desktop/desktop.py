#!/usr/bin/env python3
"""Small ACOS desktop shell and allow-listed .desktop application launcher."""

from __future__ import annotations

import configparser
import hashlib
import json
import os
import re
import shlex
import subprocess
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from urllib.parse import unquote, urlparse


ROOT = Path("/etc/acos/desktop")
HTML = ROOT / "index.html"
LOGO = ROOT / "agent-logo"
WORKSPACE = Path(os.environ.get("ACOS_WORKSPACE", "/workspace"))
PORT = 8765
FIELD_CODE = re.compile(r"%[fFuUdDnNickvm]")


def desktop_applications() -> dict[str, dict[str, object]]:
    apps: dict[str, dict[str, object]] = {}
    roots = [
        Path.home() / ".local/share/applications",
        Path("/run/current-system/sw/share/applications"),
    ]
    roots.extend(Path(p) / "applications" for p in os.environ.get("XDG_DATA_DIRS", "").split(":") if p)

    for root in roots:
        if not root.is_dir():
            continue
        for desktop_file in sorted(root.glob("*.desktop")):
            parser = configparser.ConfigParser(interpolation=None, strict=False)
            parser.optionxform = str
            try:
                parser.read(desktop_file, encoding="utf-8")
                entry = parser["Desktop Entry"]
                if entry.get("Type", "Application") != "Application":
                    continue
                if entry.get("Hidden", "false").lower() == "true" or entry.get("NoDisplay", "false").lower() == "true":
                    continue
                if entry.get("Terminal", "false").lower() == "true":
                    continue
                name = entry.get("Name", desktop_file.stem).strip()
                command = entry.get("Exec", "").strip()
                if not name or not command:
                    continue
                argv = shlex.split(command)
                expanded: list[str] = []
                for arg in argv:
                    if arg == "%i":
                        continue
                    if arg in {"%f", "%F", "%u", "%U"}:
                        continue
                    arg = FIELD_CODE.sub("", arg.replace("%c", name).replace("%k", str(desktop_file)))
                    if arg:
                        expanded.append(arg)
                if not expanded:
                    continue
                executable = Path(expanded[0]).name
                if executable == "footclient" or (executable == "foot" and "--server" in expanded):
                    continue
                try_exec = entry.get("TryExec")
                if try_exec and not (Path(try_exec).is_file() or shutil_which(try_exec)):
                    continue
                app_id = hashlib.sha256(str(desktop_file).encode()).hexdigest()[:16]
                apps[app_id] = {
                    "id": app_id,
                    "name": name,
                    "icon": entry.get("Icon", ""),
                    "description": entry.get("Comment", entry.get("GenericName", "")).strip(),
                    "argv": expanded,
                }
            except (OSError, KeyError, ValueError, configparser.Error):
                continue
    return apps


def application_icon(icon: str) -> Path | None:
    """Resolve a desktop-entry icon name using the freedesktop icon layout."""
    if not icon:
        return None
    candidate = Path(icon)
    if candidate.is_absolute() and candidate.is_file():
        return candidate

    data_dirs = [Path.home() / ".local/share"]
    data_dirs.extend(Path(p) for p in os.environ.get("XDG_DATA_DIRS", "/run/current-system/sw/share:/usr/share").split(":") if p)
    icon_names = (icon,) if candidate.suffix else tuple(f"{icon}{extension}" for extension in (".svg", ".png", ".xpm"))
    sizes = ("scalable", "512x512", "256x256", "128x128", "96x96", "64x64", "48x48", "32x32", "24x24", "16x16")

    for directory in data_dirs:
        for theme in ("hicolor", "Adwaita"):
            for size in sizes:
                for context in ("apps", "mimetypes", "devices", "categories"):
                    for icon_name in icon_names:
                        path = directory / "icons" / theme / size / context / icon_name
                        if path.is_file():
                            return path
        for icon_name in icon_names:
            path = directory / "pixmaps" / icon_name
            if path.is_file():
                return path
    return None


def image_content_type(path: Path) -> str:
    return {".svg": "image/svg+xml", ".png": "image/png", ".xpm": "image/x-xpixmap", ".jpg": "image/jpeg", ".jpeg": "image/jpeg", ".webp": "image/webp"}.get(path.suffix.lower(), "application/octet-stream")


def shutil_which(command: str) -> str | None:
    from shutil import which

    return which(command)


class DesktopHandler(BaseHTTPRequestHandler):
    server_version = "ACOSDesktop/1"

    def log_message(self, _format: str, *_args: object) -> None:
        return

    def send_bytes(self, body: bytes, content_type: str, status: int = 200) -> None:
        self.send_response(status)
        self.send_header("Content-Type", content_type)
        self.send_header("Content-Length", str(len(body)))
        self.send_header("Cache-Control", "no-store")
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self) -> None:
        path = unquote(urlparse(self.path).path)
        if path == "/":
            self.send_bytes(HTML.read_bytes(), "text/html; charset=utf-8")
        elif path == "/api/identity":
            identity = {
                "name": os.environ.get("ACOS_AGENT_NAME", "ACOS Agent"),
                "logo": LOGO.is_file(),
            }
            self.send_bytes(json.dumps(identity).encode(), "application/json")
        elif path == "/api/apps":
            public_apps = [
                {
                    "id": app["id"],
                    "name": app["name"],
                    "description": app["description"],
                    "icon": f"/api/apps/{app['id']}/icon" if application_icon(str(app["icon"])) else None,
                }
                for app in sorted(self.server.apps.values(), key=lambda item: str(item["name"]).casefold())
            ]
            self.send_bytes(json.dumps(public_apps).encode(), "application/json")
        elif path.startswith("/api/apps/") and path.endswith("/icon"):
            app_id = path.removeprefix("/api/apps/").removesuffix("/icon").strip("/")
            app = self.server.apps.get(app_id)
            icon = application_icon(str(app["icon"])) if app else None
            if icon:
                self.send_bytes(icon.read_bytes(), image_content_type(icon))
            else:
                self.send_error(404)
        elif path == "/agent-logo" and LOGO.is_file():
            data = LOGO.read_bytes()
            self.send_bytes(data, image_content_type(LOGO))
        else:
            self.send_error(404)

    def do_POST(self) -> None:
        if urlparse(self.path).path != "/api/launch":
            self.send_error(404)
            return
        if self.headers.get("Origin") != f"http://127.0.0.1:{PORT}":
            self.send_error(403)
            return
        if not self.headers.get("Content-Type", "").startswith("application/json"):
            self.send_error(415)
            return
        try:
            length = int(self.headers.get("Content-Length", "0"))
            if length < 1 or length > 4096:
                raise ValueError("invalid request size")
            request = json.loads(self.rfile.read(length))
            app = self.server.apps.get(str(request.get("id", "")))
            if app is None:
                raise ValueError("unknown application")
            argv = list(app["argv"])
            if "chromium" in Path(argv[0]).name:
                argv.extend([
                    "--new-window",
                    "--ozone-platform=wayland",
                    "--user-data-dir=/home/agent/.local/share/acos/browser",
                ])
            subprocess.Popen(argv, cwd=WORKSPACE if WORKSPACE.is_dir() else "/home/agent", start_new_session=True)
            self.send_bytes(b'{"ok":true}', "application/json")
        except (ValueError, TypeError, json.JSONDecodeError, OSError) as error:
            self.send_bytes(json.dumps({"ok": False, "error": str(error)}).encode(), "application/json", 400)


def main() -> None:
    os.environ.setdefault("XDG_CURRENT_DESKTOP", "ACOS")
    os.environ.setdefault("XDG_SESSION_TYPE", "wayland")
    apps = desktop_applications()
    server = ThreadingHTTPServer(("127.0.0.1", PORT), DesktopHandler)
    server.apps = apps

    browser = os.environ["ACOS_CHROMIUM"]
    profile = "/home/agent/.local/share/acos/desktop-browser"
    Path(profile).mkdir(parents=True, exist_ok=True)
    subprocess.Popen([
        browser,
        "--ozone-platform=wayland",
        "--enable-features=UseOzonePlatform",
        "--kiosk",
        f"--app=http://127.0.0.1:{PORT}/",
        "--no-first-run",
        "--no-default-browser-check",
        "--disable-session-crashed-bubble",
        f"--user-data-dir={profile}",
    ], start_new_session=True)
    server.serve_forever()


if __name__ == "__main__":
    main()
