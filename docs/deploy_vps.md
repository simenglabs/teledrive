# 🚀 Deploy MengDrive ke VPS

Panduan lengkap deploy `simenglabs/teledrive` ke VPS (Ubuntu/Debian) pakai Docker Compose.
Tidak perlu Rust, tidak perlu source code — cukup Docker + file compose + `.env`.

---

## Prasyarat

- VPS: 1 vCPU / 1GB RAM sudah cukup (Ubuntu 22.04/24.04 atau Debian 12)
- Domain (opsional, untuk HTTPS via reverse proxy)
- Akses SSH root / sudo

---

## 1. Install Docker (kalau belum ada)

```bash
curl -fsSL https://get.docker.com | sh
sudo usermod -aG docker $USER   # logout-login biar docker jalan tanpa sudo
docker --version && docker compose version
```

---

## 2. Siapkan direktori & konfigurasi

```bash
mkdir -p ~/mengdrive && cd ~/mengdrive

# Ambil docker-compose.yml + .env.example dari repo
curl -O https://raw.githubusercontent.com/simenglabs/teledrive/master/docker-compose.yml
curl -o .env.example https://raw.githubusercontent.com/simenglabs/teledrive/master/.env.example

cp .env.example .env
nano .env
```

Isi `.env` minimal ini:

```env
TELEGRAM_API_ID=1234567              # dari https://my.telegram.org
TELEGRAM_API_HASH=xxxxxxxxxxxxxxxx   # dari https://my.telegram.org
MASTER_ENCRYPTION_KEY=<isi `openssl rand -hex 32`>
MENGDRIVE_PORT=3060
```

> `TURSO_DATABASE_URL` default `file:/data/local.db` → metadata disimpan di volume Docker.

---

## 3. Jalankan

```bash
docker compose pull      # tarik image simenglabs/teledrive:latest
docker compose up -d

docker compose logs -f   # pantau log (Ctrl+C untuk keluar)
```

Cek sehat:

```bash
curl -I http://127.0.0.1:3060/
# HTTP/1.1 200 OK
```

---

## 4. Login Telegram (wajib, sekali saja)

Buka browser: `http://<IP-VPS>:3060/login`

1. Masukkan nomor Telegram (format internasional, mis. `+628xxxxxxxxxx`)
2. Masukkan kode OTP yang dikirim ke aplikasi Telegram
3. Setelah sukses → otomatis diarahkan ke dashboard `/ui`

Session tersimpan di database (volume `/data`) → **tetap login walau container restart**.
Nomor ini jadi "hard drive" kamu — semua file disimpan ke Saved Messages akun tersebut.

---

## 5. HTTPS + domain (opsional tapi disarankan)

OTP session cookie memakai `SameSite=Lax` — bekerja via HTTP, tapi HTTPS tetap wajib untuk produksi.

### Caddy (paling gampang, auto-SSL)

```bash
sudo apt install -y caddy
```

`/etc/caddy/Caddyfile`:

```
drive.example.com {
    reverse_proxy 127.0.0.1:3060
}
```

```bash
sudo systemctl reload caddy
```

### Nginx (alternatif)

```nginx
server {
    server_name drive.example.com;
    client_max_body_size 0;          # penting: upload file besar
    location / {
        proxy_pass http://127.0.0.1:3060;
        proxy_http_version 1.1;
        proxy_set_header Host $host;
        proxy_set_header X-Forwarded-Proto $scheme;
        proxy_buffering off;          # penting: streaming video 206
    }
}
```

```bash
sudo apt install -y certbot python3-certbot-nginx
sudo certbot --nginx -d drive.example.com
```

> `client_max_body_size 0` (Nginx) itu krusial — default Nginx cuma 1MB, upload pasti gagal.

---

## 6. Update ke versi terbaru

```bash
cd ~/mengdrive
docker compose pull
docker compose up -d     # recreate container dengan image baru
```

Data aman: metadata & session ada di volume `mengdrive_data`.

---

## 7. Perintah harian

```bash
docker compose ps              # status
docker compose logs -f         # log realtime
docker compose restart         # restart
docker compose down            # stop (data tetap di volume)
docker compose down -v         # ⚠️ HAPUS volume — session & DB hilang
docker compose pull && docker compose up -d   # update
```

---

## 8. Troubleshooting

| Gejala | Sebab & solusi |
|---|---|
| `AUTH_KEY_UNREGISTERED` di log | Belum login Telegram → buka `/login`, minta OTP |
| Upload gagal lewat domain | Nginx `client_max_body_size 0` belum diset |
| Video tidak bisa seek | Pastikan `proxy_buffering off` di Nginx |
| `TURSO_DATABASE_URL` permission error | Cek volume: `docker volume inspect mengdrive_mengdrive_data` |
| Port bentrok | Ganti `MENGDRIVE_PORT` di `.env` lalu `docker compose up -d` |

---

## Arsitektur singkat

```
Browser / aws-cli / rclone
        │  HTTP(S)
        ▼
   [VPS: Docker]
   mengdrive container ── metadata ──► /data/local.db (volume)
        │
        │  MTProto (chunk 45MB, plaintext, streaming-ready)
        ▼
   Telegram Cloud (Saved Messages) ── data sebenarnya
```

- File dipecah 45MB, langsung diteruskan ke Telegram
- Download/stream memakai partial read (HTTP 206) — video bisa langsung play tanpa download penuh
