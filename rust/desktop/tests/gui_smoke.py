#!/usr/bin/env python3
# SPDX-FileCopyrightText: 2026 Deandre Thomas
# SPDX-License-Identifier: GPL-3.0-only
"""Exercise the demo app's real X11 clipboard on an isolated Xvfb display."""

import argparse
import csv
import io
import json
import os
import re
from pathlib import Path
import select
import struct
import subprocess
import zlib


def png():
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))

    width, height = 256, 128
    data = b"".join(
        b"\0" + b"".join(bytes((40, 120 + (x // 32 % 2) * 80, 220, 255)) for x in range(width))
        for _ in range(height)
    )
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(data))
        + chunk(b"IEND", b"")
    )


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("binary", type=Path)
    parser.add_argument("output", type=Path)
    parser.add_argument("--tessdata", type=Path, help="directory containing eng.traineddata")
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    for key in ("WAYLAND_DISPLAY", "XDG_RUNTIME_DIR", "XDG_SESSION_TYPE", "DBUS_SESSION_BUS_ADDRESS", "XAUTHORITY"):
        env.pop(key, None)
    env["LIBGL_ALWAYS_SOFTWARE"] = "1"
    if args.tessdata is not None:
        env["TESSDATA_PREFIX"] = str(args.tessdata.resolve())
    for variable in ("XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME"):
        directory = args.output / variable.lower()
        directory.mkdir(exist_ok=True)
        env[variable] = str(directory.resolve())
    children = []
    log = (args.output / "gui.log").open("w")

    def run(command, **kwargs):
        return subprocess.run(command, env=env, check=True, timeout=40, **kwargs)

    def xdo(*arguments):
        run(["xdotool", *map(str, arguments)], stdout=subprocess.PIPE)

    def settle():
        xdo("sleep", "0.4")

    def clipboard(data, mime):
        child = subprocess.Popen(
            ["xclip", "-selection", "clipboard", "-t", mime, "-quiet"],
            env=env, stdin=subprocess.PIPE, stdout=log, stderr=log,
        )
        children.append(child)
        child.stdin.write(data)
        child.stdin.close()
        settle()

    def snapshot(name):
        settle()
        path = args.output / f"{name}.png"
        run(["import", "-window", window, str(path)])
        result = run(["tesseract", str(path), "stdout", "--psm", "11", "-c", "tessedit_create_tsv=1"],
                     stdout=subprocess.PIPE, stderr=log, text=True)
        words = list(csv.DictReader(io.StringIO(result.stdout), delimiter="\t"))
        text = " ".join(word["text"] for word in words)
        (args.output / f"{name}.txt").write_text(text)
        return words, text

    def word(words, text):
        for item in words:
            if re.sub(r"^\W+|\W+$", "", item["text"]) == text:
                return int(item["left"]), int(item["top"]), int(item["height"])
        raise AssertionError(f"Cannot find {text!r} in screenshot")

    def click_word(words, text):
        x, y, height = word(words, text)
        xdo("mousemove", "--window", window, x + 8, y + height // 2, "click", 1)

    try:
        read_fd, write_fd = os.pipe()
        xvfb = subprocess.Popen(
            ["Xvfb", "-displayfd", str(write_fd), "-screen", "0", "1152x900x24", "-nolisten", "tcp"],
            pass_fds=(write_fd,), stdout=log, stderr=log,
        )
        children.append(xvfb)
        os.close(write_fd)
        with os.fdopen(read_fd) as display_pipe:
            if not select.select([read_fd], [], [], 30)[0]:
                raise TimeoutError("Xvfb did not assign a display")
            display = display_pipe.readline().strip()
        assert display.isdigit(), "Xvfb did not provide a display number"
        env["DISPLAY"] = f":{display}"
        app = subprocess.Popen([str(args.binary.resolve()), "--demo"], env=env, stdout=log, stderr=log)
        children.append(app)
        window = run(["xdotool", "search", "--sync", "--onlyvisible", "--name", "^KDE Connect Rust$"],
                     stdout=subprocess.PIPE, text=True).stdout.splitlines()[0]
        xdo("windowfocus", "--sync", window)
        words, text = snapshot("initial")
        assert "DEMO MODE" in text, text
        x, y, height = word(words, "Recipient")
        xdo("mousemove", "--window", window, x + 60, y + height + 14, "click", 1)
        xdo("type", "+15550100001")
        x, y, height = word(words, "Message")
        xdo("mousemove", "--window", window, x + 60, y + height + 14, "click", 1)
        clipboard(b"Clipboard text survives", "UTF8_STRING")
        xdo("key", "ctrl+v")
        words, text = snapshot("text-paste")
        assert "Clipboard text survives" in text, text
        clipboard(png(), "image/png")
        xdo("key", "ctrl+v")
        words, text = snapshot("image-paste")
        assert "Image attached" in text and "Copy image" in text, text
        click_word(words, "Copy")
        settle()
        copied = run(["xclip", "-selection", "clipboard", "-t", "image/png", "-o"], stdout=subprocess.PIPE).stdout
        assert copied.startswith(b"\x89PNG\r\n\x1a\n")
        assert struct.unpack(">II", copied[16:24]) == (256, 128)
        words, text = snapshot("image-copy")
        assert "Image copied to clipboard" in text, text
        click_word(words, "Review")
        words, text = snapshot("confirm")
        assert "Confirm message" in text, text
        click_word(words, "Cancel")
        words, text = snapshot("cancelled")
        assert "Clipboard text survives" in text and "Copy image" in text, text
        click_word(words, "Review")
        words, text = snapshot("confirm-again")
        click_word(words, "Simulate")
        words, text = snapshot("demo-sent")
        assert "Demo send completed" in text and "Copy image" not in text, text
        result = {"passed": True, "checks": ["text paste", "Ctrl+V image paste", "image copy", "cancel preserves draft", "demo send clears draft"]}
        (args.output / "result.json").write_text(json.dumps(result, indent=2))
        print(json.dumps(result))
    finally:
        for child in reversed(children):
            if child.poll() is None:
                child.terminate()
                try:
                    child.wait(timeout=10)
                except subprocess.TimeoutExpired:
                    child.kill()
                    child.wait()
        log.close()


if __name__ == "__main__":
    main()
