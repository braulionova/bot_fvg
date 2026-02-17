#!/usr/bin/env python3
"""
Descarga 4 años de velas 4H desde Bybit (API pública, sin auth)
y guarda un CSV por símbolo en la carpeta ./data/
"""

import csv
import json
import os
import time
import urllib.request
from datetime import datetime, timezone

# ── Configuración ─────────────────────────────────────────────────────────────
SYMBOLS   = ["BTCUSDT", "ETHUSDT", "BNBUSDT", "XRPUSDT", "SOLUSDT"]
INTERVAL  = 240          # 4 horas en minutos
LIMIT     = 1000         # máximo permitido por Bybit
YEARS     = 4
BASE_URL  = "https://api.bybit.com"
DATA_DIR  = os.path.join(os.path.dirname(__file__), "data")

# ── Helpers ────────────────────────────────────────────────────────────────────
def now_ms() -> int:
    return int(datetime.now(timezone.utc).timestamp() * 1000)

def ts_ms(dt: datetime) -> int:
    return int(dt.timestamp() * 1000)

def fmt(ms: int) -> str:
    return datetime.utcfromtimestamp(ms / 1000).strftime("%Y-%m-%d %H:%M")

def fetch_klines(symbol: str, start_ms: int, end_ms: int) -> list:
    url = (
        f"{BASE_URL}/v5/market/kline"
        f"?category=linear&symbol={symbol}"
        f"&interval={INTERVAL}&limit={LIMIT}"
        f"&start={start_ms}&end={end_ms}"
    )
    try:
        with urllib.request.urlopen(url, timeout=15) as resp:
            data = json.loads(resp.read())
        if data.get("retCode") != 0:
            print(f"  ⚠  API error: {data.get('retMsg')}")
            return []
        # list: [[timestamp, open, high, low, close, volume, turnover], ...]
        return data["result"]["list"]
    except Exception as e:
        print(f"  ⚠  Request failed: {e}")
        return []

# ── Descarga por símbolo ───────────────────────────────────────────────────────
def download_symbol(symbol: str):
    from datetime import timedelta

    end_ms   = now_ms()
    start_ms = ts_ms(datetime.now(timezone.utc) - timedelta(days=365 * YEARS))

    out_path = os.path.join(DATA_DIR, f"{symbol}_4H.csv")
    all_rows: list[list] = []

    print(f"\n{'─'*55}")
    print(f"  {symbol}  {fmt(start_ms)}  →  {fmt(end_ms)}")
    print(f"{'─'*55}")

    chunk_end = end_ms
    requests  = 0

    while chunk_end > start_ms:
        rows = fetch_klines(symbol, start_ms, chunk_end)
        requests += 1

        if not rows:
            break

        # Bybit devuelve de más reciente → más antiguo
        rows_sorted = sorted(rows, key=lambda r: int(r[0]))
        all_rows = rows_sorted + all_rows

        oldest_ts = int(rows_sorted[0][0])
        print(f"  [{requests:>3}]  fetched {len(rows):>4} candles  "
              f"oldest={fmt(oldest_ts)}  total={len(all_rows)}", end="\r")

        if oldest_ts <= start_ms:
            break

        chunk_end = oldest_ts - 1
        time.sleep(0.15)   # evitar rate-limit (600 req/min en Bybit)

    print(f"\n  ✓  {len(all_rows)} velas totales  ({requests} requests)")

    # Filtrar estrictamente al rango pedido
    all_rows = [r for r in all_rows if start_ms <= int(r[0]) <= end_ms]

    # Escribir CSV
    with open(out_path, "w", newline="") as f:
        writer = csv.writer(f)
        writer.writerow(["timestamp_ms", "datetime_utc",
                         "open", "high", "low", "close", "volume", "turnover"])
        for r in all_rows:
            ts   = int(r[0])
            dt   = datetime.utcfromtimestamp(ts / 1000).strftime("%Y-%m-%d %H:%M")
            writer.writerow([ts, dt, r[1], r[2], r[3], r[4], r[5], r[6]])

    size_kb = os.path.getsize(out_path) / 1024
    print(f"  📄 Guardado: {out_path}  ({size_kb:.1f} KB)")

# ── Main ───────────────────────────────────────────────────────────────────────
def main():
    os.makedirs(DATA_DIR, exist_ok=True)

    print(f"\n🚀 Descargando {YEARS} años de velas 4H para {len(SYMBOLS)} pares")
    print(f"   Destino: {DATA_DIR}\n")

    for symbol in SYMBOLS:
        download_symbol(symbol)

    print(f"\n{'═'*55}")
    print("✅ Descarga completa.")
    print(f"{'═'*55}\n")

    # Resumen
    for symbol in SYMBOLS:
        path = os.path.join(DATA_DIR, f"{symbol}_4H.csv")
        if os.path.exists(path):
            with open(path) as f:
                lines = sum(1 for _ in f) - 1  # excluir header
            kb = os.path.getsize(path) / 1024
            print(f"  {symbol:<12} {lines:>6} velas   {kb:>7.1f} KB   {path}")

    print()

if __name__ == "__main__":
    main()
