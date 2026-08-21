#!/usr/bin/env python3
"""
Issue and package the Developer ID Application certificate, without anybody
opening Keychain Assistant.

Signing a Mac app needs four things in CI: a certificate, the private key that
goes with it, the two bundled into a .p12, and that .p12 in GitHub's secrets.
Done by hand that is a Certificate Assistant wizard, a browser upload, a
Keychain export and four copy-pastes, and every one of them is a step somebody
gets subtly wrong once a year when the certificate expires.

All of it is automatable except one thing: **somebody with Account Holder
rights has to create an App Store Connect API key in a browser, once.** That
is Apple's security model rather than a gap in the tooling - credentials
cannot be bootstrapped from nothing. This script needs that key and automates
everything after it.

The same API key also authenticates notarization, which is why this repo needs
no app-specific password. An app-specific password is tied to one person's
Apple ID and their second factor; a team key is not, and it can be rotated
without a human being available.

## Using it

    # 1. Prove the key works and see what already exists
    scripts/apple-signing.py check --key <path.p8> --key-id <id> --issuer <uuid>

    # 2. Generate a key pair, ask Apple for the certificate, build the .p12
    scripts/apple-signing.py issue --key <path.p8> --key-id <id> --issuer <uuid>

    # 3. If step 2 says Apple refused, upload the CSR it left behind and then
    scripts/apple-signing.py package --cer <downloaded.cer> --private-key <key.pem>

`issue` prints the `gh secret set` commands rather than running them, so the
values are visible to the person who owns the account before they are stored.

## What this never does

It does not print private key material, and it does not write the .p12
anywhere but a path you name. The passphrase it generates is random and is
meant to stay unknown to humans - only CI ever decrypts the .p12, so a
memorable password buys nothing and leaks eventually.
"""

from __future__ import annotations

import argparse
import base64
import json
import secrets
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

API = "https://api.appstoreconnect.apple.com/v1"

# What we are asking Apple to issue. Not DEVELOPER_ID_KEXT (kernel extensions)
# and not DEVELOPER_ID_INSTALLER, which signs .pkg files - we ship a .dmg
# containing a .app, and this is the type that signs it.
CERT_TYPE = "DEVELOPER_ID_APPLICATION"


def run(cmd: list[str], stdin: bytes | None = None) -> bytes:
    """Shell out, and fail loudly with the tool's own message."""
    proc = subprocess.run(cmd, input=stdin, capture_output=True)
    if proc.returncode != 0:
        sys.exit(f"{cmd[0]} failed: {proc.stderr.decode(errors='replace').strip()}")
    return proc.stdout


def b64url(raw: bytes) -> str:
    return base64.urlsafe_b64encode(raw).rstrip(b"=").decode()


def der_to_raw(der: bytes) -> bytes:
    """
    An ECDSA signature, as JWT wants it.

    OpenSSL emits `SEQUENCE { INTEGER r, INTEGER s }`; ES256 wants the two
    integers concatenated, each left-padded to exactly 32 bytes. Getting this
    wrong produces a token Apple rejects as malformed with no hint as to why,
    so it is parsed properly rather than sliced.
    """
    if der[0] != 0x30:
        sys.exit("the signature is not a DER SEQUENCE")
    # Skip the SEQUENCE header, whose length byte may be long-form.
    i = 2 if der[1] < 0x80 else 2 + (der[1] & 0x7F)

    def integer(at: int) -> tuple[int, bytes]:
        if der[at] != 0x02:
            sys.exit("expected a DER INTEGER in the signature")
        length = der[at + 1]
        value = der[at + 2 : at + 2 + length]
        # DER keeps a leading zero to mark the number positive; JWT does not.
        return at + 2 + length, value.lstrip(b"\x00").rjust(32, b"\x00")

    i, r = integer(i)
    _, s = integer(i)
    return r + s


def token(p8: Path, key_id: str, issuer: str | None) -> str:
    """
    A short-lived ES256 token for the App Store Connect API.

    Two shapes exist and which one is correct depends on how the key was
    created, which the .p8 file does not record. A **team** key names its
    issuer; an **individual** key has no issuer and identifies itself with
    `sub: "user"`. Passing no `--issuer` selects the second, so a key of
    either kind works without the caller having to know the distinction.
    """
    header = {"alg": "ES256", "kid": key_id, "typ": "JWT"}
    now = int(time.time())
    payload: dict[str, object] = {
        "iat": now,
        # Twenty minutes is Apple's ceiling for this audience. There is no
        # reason to ask for less; the token never leaves this process.
        "exp": now + 20 * 60,
        "aud": "appstoreconnect-v1",
    }
    if issuer:
        payload["iss"] = issuer
    else:
        payload["sub"] = "user"

    signing_input = f"{b64url(json.dumps(header).encode())}.{b64url(json.dumps(payload).encode())}"
    der = run(["openssl", "dgst", "-sha256", "-sign", str(p8)], stdin=signing_input.encode())
    return f"{signing_input}.{b64url(der_to_raw(der))}"


def call(method: str, path: str, jwt: str, body: dict | None = None) -> dict:
    request = urllib.request.Request(
        f"{API}{path}",
        method=method,
        data=json.dumps(body).encode() if body else None,
        headers={
            "Authorization": f"Bearer {jwt}",
            **({"Content-Type": "application/json"} if body else {}),
        },
    )
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            return json.loads(response.read() or b"{}")
    except urllib.error.HTTPError as e:
        detail = e.read().decode(errors="replace")
        try:
            errors = json.loads(detail).get("errors", [])
            detail = "\n".join(
                f"  {x.get('title')}: {x.get('detail')}" for x in errors
            ) or detail
        except Exception:
            pass
        raise SystemExit(f"Apple answered {e.code}:\n{detail}")


def jwt_from(args) -> str:
    p8 = Path(args.key).expanduser()
    if not p8.is_file():
        sys.exit(f"no such key file: {p8}")
    return token(p8, args.key_id, args.issuer)


def cmd_check(args) -> None:
    """Prove the key authenticates, and show what already exists."""
    data = call("GET", "/certificates?limit=200", jwt_from(args)).get("data", [])
    print(f"authenticated. {len(data)} certificate(s) on the account:\n")
    developer_id = 0
    for cert in data:
        attrs = cert.get("attributes", {})
        kind = attrs.get("certificateType", "?")
        if kind == CERT_TYPE:
            developer_id += 1
        print(
            f"  {kind:<32} {attrs.get('displayName', '?'):<34} "
            f"expires {attrs.get('expirationDate', '?')[:10]}"
        )
    print(
        f"\n{developer_id} of the 5 permitted {CERT_TYPE} certificates are in use."
        if developer_id
        else f"\nNo {CERT_TYPE} certificate exists yet. `issue` creates one."
    )


def cmd_issue(args) -> None:
    out = Path(args.out).expanduser()
    out.mkdir(parents=True, exist_ok=True)
    key_pem, csr_pem = out / "developer-id.key.pem", out / "developer-id.csr"

    # The private key is generated here and never leaves this machine except
    # inside the .p12. Apple only ever sees the certificate request.
    print("generating a private key and certificate request...")
    run(["openssl", "req", "-new", "-newkey", "rsa:2048", "-nodes",
         "-keyout", str(key_pem), "-out", str(csr_pem),
         "-subj", f"/CN={args.common_name}/O={args.org}/C={args.country}"])
    key_pem.chmod(0o600)

    print(f"asking Apple for a {CERT_TYPE} certificate...")
    try:
        answer = call("POST", "/certificates", jwt_from(args), {
            "data": {
                "type": "certificates",
                "attributes": {
                    "certificateType": CERT_TYPE,
                    "csrContent": csr_pem.read_text(),
                },
            }
        })
    except SystemExit as refusal:
        # The documented fallback. Apple has restricted this endpoint before,
        # and a refusal here costs nothing: the request is already written and
        # the browser accepts the same file.
        print(f"\n{refusal}\n")
        print("Apple would not issue it through the API. The certificate request is at:")
        print(f"  {csr_pem}")
        print("\nUpload it at https://developer.apple.com/account/resources/certificates/add")
        print(f"(Software -> Developer ID -> Developer ID Application), then run:\n")
        print(f"  {sys.argv[0]} package --cer ~/Downloads/developerID_application.cer \\")
        print(f"      --private-key {key_pem}")
        raise SystemExit(1)

    cert_b64 = answer["data"]["attributes"]["certificateContent"]
    cer = out / "developer-id.cer"
    cer.write_bytes(base64.b64decode(cert_b64))
    print(f"issued: {answer['data']['attributes'].get('displayName')}")
    package(cer, key_pem, out)


def cmd_package(args) -> None:
    key = Path(args.private_key).expanduser()
    package(Path(args.cer).expanduser(), key, key.parent)


def package(cer: Path, key_pem: Path, out: Path) -> None:
    """Bundle certificate, key and Apple's intermediate into a .p12."""
    cert_pem = out / "developer-id.cert.pem"
    # A .cer downloaded from Apple is DER; everything downstream wants PEM.
    head = cer.read_bytes()[:1]
    if head == b"0":  # DER SEQUENCE
        run(["openssl", "x509", "-inform", "DER", "-in", str(cer), "-out", str(cert_pem)])
    else:
        cert_pem.write_bytes(cer.read_bytes())

    # Apple's intermediate, so `codesign` can build a full chain on a runner
    # whose keychain has never seen one. It is already on any Mac that has
    # Xcode, so take it from there rather than fetching it over the network.
    chain = out / "apple-intermediate.pem"
    found = subprocess.run(
        ["security", "find-certificate", "-a", "-c", "Developer ID Certification Authority", "-p"],
        capture_output=True,
    )
    extra: list[str] = []
    if found.returncode == 0 and b"BEGIN CERTIFICATE" in found.stdout:
        chain.write_bytes(found.stdout)
        extra = ["-certfile", str(chain)]
    else:
        print("note: no Developer ID intermediate in the keychain; the .p12 "
              "will carry the leaf alone, which usually still verifies")

    passphrase = secrets.token_urlsafe(24)
    p12 = out / "developer-id.p12"
    run([
        "openssl", "pkcs12", "-export",
        "-inkey", str(key_pem), "-in", str(cert_pem), *extra,
        "-out", str(p12), "-passout", f"pass:{passphrase}",
        # macOS `security import` is fussy about newer PKCS#12 algorithms, and
        # the failure is an opaque "MAC verification failed" in CI. These are
        # the widely-compatible ones.
        "-legacy", "-keypbe", "PBE-SHA1-3DES", "-certpbe", "PBE-SHA1-3DES", "-macalg", "sha1",
    ])
    p12.chmod(0o600)

    subject = run(["openssl", "x509", "-in", str(cert_pem), "-noout", "-subject"]).decode()
    identity = subject.split("CN=", 1)[1].split(", OU=", 1)[0].strip() if "CN=" in subject else "?"
    team = subject.split("OU=", 1)[1].split(",", 1)[0].strip() if "OU=" in subject else "?"

    print(f"\nbuilt {p12}\n")
    print("Set these, then delete the working directory:\n")
    repo = "kitchen-table/kitchentable"
    print(f"  base64 -i {p12} | gh secret set APPLE_CERTIFICATE --repo {repo}")
    print(f"  printf %s '{passphrase}' | gh secret set APPLE_CERTIFICATE_PASSWORD --repo {repo}")
    print(f"  printf %s '{identity}' | gh secret set APPLE_SIGNING_IDENTITY --repo {repo}")
    print(f"  printf %s '{team}' | gh secret set APPLE_TEAM_ID --repo {repo}")
    print(f"\nBack up {p12} and its passphrase somewhere durable first.")
    print("Losing the private key means burning one of five Developer ID certificates.")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[1])
    subs = parser.add_subparsers(dest="command", required=True)

    def api_args(p):
        p.add_argument("--key", required=True, help="path to the AuthKey_*.p8")
        p.add_argument("--key-id", required=True, help="the key's ID")
        p.add_argument("--issuer", help="issuer UUID; omit for an individual key")

    api_args(subs.add_parser("check", help="prove the key works and list certificates"))

    issue = subs.add_parser("issue", help="create the certificate and the .p12")
    api_args(issue)
    issue.add_argument("--org", default="Nola AI Ltd")
    issue.add_argument("--common-name", default="Kitchen Table Developer ID")
    issue.add_argument("--country", default="GB")
    issue.add_argument("--out", default="./.signing", help="working directory")

    pkg = subs.add_parser("package", help="build the .p12 from a manually issued .cer")
    pkg.add_argument("--cer", required=True)
    pkg.add_argument("--private-key", required=True)

    args = parser.parse_args()
    {"check": cmd_check, "issue": cmd_issue, "package": cmd_package}[args.command](args)


if __name__ == "__main__":
    main()
