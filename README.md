# 🦀🦀🦀 Autoforward 🦀🦀🦀
En applikasjon som automagisk router .nais.preprod.local ingresser til Kubernetes clusteret via kubectl

### Generer sertifikat for https
```
openssl req -x509 -newkey rsa:4096 -nodes -keyout key.pem -out cert.pem -days 3650
```