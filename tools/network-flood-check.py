#!/usr/bin/env python3
"""Flood a local magicka-server and check that it survives and stays joinable.

Only ever targets 127.0.0.1, on a server this script starts itself. Junk comes
from many source ports, because each new source address costs the host a link.
Reports memory and whether a real client can join during and after the flood.

usage: network-flood-check.py <magicka-server> <physics-client-probe> <content> [seconds]
"""
import os
import re
import socket
import subprocess
import sys
import threading
import time


def rss_kb(pid):
    with open(f"/proc/{pid}/status") as status:
        for line in status:
            if line.startswith("VmRSS:"):
                return int(line.split()[1])
    return 0


def flood(port, stop, counters):
    payloads = [
        os.urandom(64),
        os.urandom(1200),
        # Looks like a netcode connection request: first byte zero.
        b"\x00" + os.urandom(1077),
        # A garbage keep-alive/payload prefix.
        b"\x05" + os.urandom(200),
    ]
    sockets = []
    while not stop.is_set():
        # A fresh source port per batch makes the host spawn new links.
        sock = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
        sock.setblocking(False)
        sockets.append(sock)
        for payload in payloads:
            try:
                sock.sendto(payload, ("127.0.0.1", port))
                counters[0] += 1
            except OSError:
                pass
        if len(sockets) > 512:
            sockets.pop(0).close()
    for sock in sockets:
        sock.close()


def handshake_flood(tool, address, seconds, totals):
    result = subprocess.run(
        [tool, "--handshake-flood", address, str(seconds)],
        capture_output=True,
        text=True,
    )
    totals.append(int(result.stdout.strip() or 0))


def probe(tool, address, seconds):
    """Time until a real client receives game state, or None."""
    started = time.monotonic()
    process = subprocess.Popen(
        [tool, "--join", address],
        stdout=subprocess.PIPE,
        stderr=subprocess.DEVNULL,
    )
    try:
        # The probe prints one line once it has game state.
        result = [None]

        def read():
            if process.stdout.readline():
                result[0] = time.monotonic() - started

        reader = threading.Thread(target=read, daemon=True)
        reader.start()
        reader.join(seconds)
        return result[0]
    finally:
        process.kill()
        process.wait()


def main():
    server_bin, probe_bin, content = sys.argv[1:4]
    seconds = int(sys.argv[4]) if len(sys.argv) > 4 else 60
    # FLOOD_FROM=<this computer's LAN address>: attack through that address, so
    # the flood comes from a non-loopback source while the player stays on
    # 127.0.0.1, a different address. MAGICKA_PASSPHRASE makes the game private;
    # the player has it, the flood does not.
    attack_ip = os.environ.get("FLOOD_FROM")
    bind = "0.0.0.0:0" if attack_ip else "127.0.0.1:0"
    server = subprocess.Popen(
        [server_bin, "--content", content, "--level", "Havindr.lvl", "--bind", bind, "--no-scripts"],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
    )
    failed = False
    try:
        line = ""
        while "listening on" not in line:
            line = server.stdout.readline()
            if not line:
                sys.exit("server did not start")
        address = re.search(r"listening on (\S+)", line).group(1)
        port = int(address.rsplit(":", 1)[1])
        target = f"{attack_ip}:{port}" if attack_ip else address
        address = f"127.0.0.1:{port}"
        log = sys.argv[5] if len(sys.argv) > 5 else os.devnull

        def keep_output():
            with open(log, "w") as out:
                for text in server.stdout:
                    out.write(text)
                    out.flush()

        threading.Thread(target=keep_output, daemon=True).start()
        print(f"server {address}, idle RSS {rss_kb(server.pid)//1024} MiB")
        print(f"baseline join: {probe(probe_bin, address, 20)}")

        stop = threading.Event()
        counters = [0]
        started_handshakes = []
        if os.environ.get("FLOOD_KIND") == "handshake":
            # Valid connect tokens, abandoned: what open games cannot refuse.
            threads = [
                threading.Thread(
                    target=handshake_flood,
                    args=(probe_bin, target, seconds, started_handshakes),
                )
                for _ in range(3)
            ]
        else:
            threads = [threading.Thread(target=flood, args=(port, stop, counters)) for _ in range(2)]
        for thread in threads:
            thread.start()
        started = time.monotonic()
        peak = 0
        next_probe = 10
        joined = []
        while time.monotonic() - started < seconds:
            time.sleep(1)
            elapsed = time.monotonic() - started
            peak = max(peak, rss_kb(server.pid))
            if server.poll() is not None:
                print(f"SERVER DIED after {elapsed:.0f}s, exit {server.returncode}")
                failed = True
                break
            if elapsed >= next_probe:
                joined.append(probe(probe_bin, address, 20))
                next_probe = elapsed + 15
        stop.set()
        for thread in threads:
            thread.join()
        rate = counters[0] / max(time.monotonic() - started, 1)
        if started_handshakes:
            print(f"abandoned handshakes started: {sum(started_handshakes)}")
        print(f"flood: {counters[0]} datagrams, {rate:.0f}/s, peak RSS {peak//1024} MiB")
        print(f"joins during flood (seconds, None = failed): {joined}")
        if not failed:
            time.sleep(15)
            after = probe(probe_bin, address, 20)
            print(f"join after flood: {after}, RSS {rss_kb(server.pid)//1024} MiB")
            failed = after is None or server.poll() is not None
    finally:
        server.kill()
        server.wait()
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
