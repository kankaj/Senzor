Test-only TLS material for `senzor-core`'s unit tests: a self-signed CA (`ca.pem`), and a
client certificate signed by it (`client.pem`) with its key (`client.key`). Nothing here is a
secret; the CA's own key was discarded after signing.
