# Tries a frozen zotify (release.yml, on the machine that built it): it must start, and with a
# made-up login get as far as logging in to Spotify. A part PyInstaller left out shows up as a
# missing module before that.
set -uo pipefail
exe="$1"
help=$("$exe" --help 2>&1) || { echo "$help"; echo "zotify --help failed"; exit 1; }
echo "$help" | head -1
case "$help" in *"usage: zotify"*) ;; *) echo "zotify --help says something else"; exit 1 ;; esac

printf '{"username":"nobody","credentials":"AAAA","type":"AUTHENTICATION_STORED_SPOTIFY_CREDENTIALS"}' > fake-login.json
"$exe" --no-splash --config-location=zconfig.json --credentials-location=fake-login.json \
  --save-credentials=False --root-path=zout --song-archive=zarchive \
  https://open.spotify.com/track/4uLU6hMCjMI75M1A2tKUQC < /dev/null > out.txt 2>&1 &
pid=$!
for _ in $(seq 1 120); do
  kill -0 "$pid" 2> /dev/null || break
  sleep 1
done
if kill -0 "$pid" 2> /dev/null; then
  # Still talking to Spotify after two minutes: everything it needed had loaded.
  kill "$pid" 2> /dev/null || true
  echo "zotify was still logging in after 2 minutes (stopped)" >> out.txt
fi
wait "$pid" 2> /dev/null || true
tail -6 out.txt
if grep -qE "ModuleNotFoundError|ImportError|No module named" out.txt; then
  echo "zotify is missing part of itself"
  exit 1
fi
if ! grep -qE "Traceback|Username|Authentication|still logging in" out.txt; then
  echo "zotify didn't get as far as logging in"
  exit 1
fi
echo "zotify works"
