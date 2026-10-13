"""Starts KWallet in the current D-Bus session the way a KDE login does.

pam_kwallet hands kwalletd5 a key derived from the login password, and
kwalletd5 opens the default wallet with it, creating the wallet when it does
not exist, without the first-use dialog. This script plays pam_kwallet's part
for the credential store job of ci.yml (V7), so that the Secret Service of
KWallet serves an open default collection. The protocol is the one of
kwalletd5 5.115 (src/runtime/kwalletd/main.cpp): with PAM_KWALLET5_LOGIN set,
`--pam-login <pipe> <socket>` reads the 56-byte key from the pipe, then accepts
one connection on the listening socket and reads environment lines until it
closes. Python is used because it can hand a listening socket to a child
process, which Node.js cannot do through its public API. The test machine is
disposable.
"""

import hashlib
import os
import socket
import subprocess
import sys
import tempfile
import time

# kwalletd's PBKDF2-SHA512 parameters (PBKDF2_SHA512_KEYSIZE, PBKDF2_SHA512_ITERATIONS).
KEY_SIZE = 56
ITERATIONS = 50000
PASSWORD = b"mannie-ci"
SECRET_SERVICE = "org.freedesktop.secrets"


def write_settings() -> None:
    """Enables the wallet and its Secret Service API, skips the first-use
    wizard, and lets the Secret Service use the open wallet. KWallet opens the
    wallet for Secret Service clients as "KDE System" and asks the user once
    before it allows that; the entry records the answer "Allow always"."""
    config = os.path.join(os.path.expanduser("~"), ".config")
    os.makedirs(config, exist_ok=True)
    with open(os.path.join(config, "kwalletrc"), "w", encoding="utf-8") as settings:
        settings.write(
            "[Wallet]\nEnabled=true\nFirst Use=false\n\n"
            "[Auto Allow]\nkdewallet=KDE System\n\n"
            "[org.freedesktop.secrets]\napiEnabled=true\n"
        )


def login_key() -> bytes:
    """The key pam_kwallet derives from the password and the wallet's salt file."""
    folder = os.path.join(os.path.expanduser("~"), ".local", "share", "kwalletd")
    os.makedirs(folder, exist_ok=True)
    salt_path = os.path.join(folder, "kdewallet.salt")
    # Like pam_kwallet, keep the salt of an existing wallet: a new salt would
    # give a key that no longer opens it.
    if os.path.exists(salt_path):
        with open(salt_path, "rb") as salt_file:
            salt = salt_file.read()
    else:
        salt = os.urandom(KEY_SIZE)
        with open(salt_path, "wb") as salt_file:
            salt_file.write(salt)
    return hashlib.pbkdf2_hmac("sha512", PASSWORD, salt, ITERATIONS, dklen=KEY_SIZE)


def secret_service_is_up() -> bool:
    reply = subprocess.run(
        [
            "dbus-send",
            "--session",
            "--print-reply",
            "--dest=org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus.NameHasOwner",
            f"string:{SECRET_SERVICE}",
        ],
        capture_output=True,
        text=True,
        check=False,
    )
    return reply.stdout.strip().endswith("true")


def main() -> int:
    write_settings()
    key = login_key()
    key_reader, key_writer = os.pipe()
    listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
    path = os.path.join(tempfile.mkdtemp(), "kwallet-environment")
    listener.bind(path)
    listener.listen(1)
    environment = dict(os.environ, PAM_KWALLET5_LOGIN="1", QT_QPA_PLATFORM="offscreen")
    # The daemon outlives this script; its output goes to a file so that it
    # does not hold the job's output open.
    log_path = os.path.join(tempfile.gettempdir(), "kwalletd5.log")
    with open(log_path, "wb") as log:
        subprocess.Popen(
            ["kwalletd5", "--pam-login", str(key_reader), str(listener.fileno())],
            pass_fds=(key_reader, listener.fileno()),
            env=environment,
            stdin=subprocess.DEVNULL,
            stdout=log,
            stderr=subprocess.STDOUT,
            start_new_session=True,
        )
    os.close(key_reader)
    os.write(key_writer, key)
    os.close(key_writer)
    # The daemon inherits this process's environment, so it needs no lines.
    with socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
        client.connect(path)
    listener.close()
    deadline = time.monotonic() + 30
    while not secret_service_is_up():
        if time.monotonic() > deadline:
            print(f"kwallet-login: kwalletd5 did not offer the Secret Service; see {log_path}", file=sys.stderr)
            return 1
        time.sleep(0.5)
    return 0


if __name__ == "__main__":
    sys.exit(main())
