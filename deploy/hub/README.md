# ws hub on a VPS

Install the ws hub on a new Ubuntu VPS (22.04 or later). The hub serves mTLS on port 443.

## 1. Create the VPS and the DNS record

1. Create an Ubuntu VPS (DigitalOcean or Hetzner) with a public IPv4 address and SSH access as root.
2. Open ports 80 and 443 in the VPS firewall. Port 80 is for certificate issue and renewal.
3. Add an `A` record for the hub name (for example `hub.example.com`) that points to the VPS address. Wait until `dig +short hub.example.com` shows the address. If Cloudflare manages the record, set it to "DNS only".

## 2. Run the script

Copy the folder to the VPS and run the script as root. Use a branch, a tag or a commit as `--ref`.

```sh
scp -r deploy/hub root@hub.example.com:/root/hub-deploy
scp root-ca.pem root@hub.example.com:/root/root-ca.pem
ssh root@hub.example.com
/root/hub-deploy/install-hub.sh \
  --domain hub.example.com \
  --email you@example.com \
  --ref main \
  --ca-cert /root/root-ca.pem
```

The root CA certificate is the public `root-ca.pem` that `ws ca init` wrote (step 3 below). It is not secret. Never copy the root key to the VPS.

The script does these steps in order:

1. Installs the build tools, `certbot` and `rustup`.
2. Creates the user `ws-hub` and `/var/lib/ws-hub`.
3. Installs the root CA certificate to `/etc/ws-hub/root-ca.pem`.
4. Gets the TLS certificate with `certbot --standalone`. A renew hook restarts the service.
5. Checks out `rstagi/dotfile` at the ref, builds `ws` in release mode and installs `/usr/local/bin/ws`.
6. Installs `ws-hub.service` and starts it.

To update the hub, run the script again with a new `--ref`. The second run replaces the binary and the unit and restarts the service. The state in `/var/lib/ws-hub` stays.

## 3. Copy the root CA certificate

Create the root CA on a trusted device if you have none: `ws ca init --out <dir>`. Keep the root key in 1Password. Copy only the certificate file to the VPS, as in step 2.

## 4. Add the first device

On the VPS, add the certificate of the first device. The command runs as `ws-hub` so that the files keep the right owner.

```sh
sudo -u ws-hub ws hub add-device --state /var/lib/ws-hub --cert /root/laptop-cert.pem --name laptop
```

On the device, add a `[hub]` section to `config.toml` in the state directory (`url`, `cert`, `key`, `ca`). Then check the link:

```sh
ws device list
```

## 5. Pair the second device

Use the pairing commands from Phase 153. On the new device run `ws device join <hub>`. It shows a short code. On a trusted device run `ws device approve <code>`. The hub then lists both devices in `ws device list`.

## Operate

- Status: `systemctl status ws-hub`
- Logs: `journalctl -u ws-hub -f`
- Certificate renewal: `certbot renew --dry-run`
- Back up `/var/lib/ws-hub` (it holds `hub.db`).
