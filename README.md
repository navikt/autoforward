# 🦀🦀🦀 Autoforward 🦀🦀🦀
En applikasjon som automagisk router ingresser i dev-fss til Kubernetes clusteret
via kubectl

## Hvordan ta i bruk
### Bygg applikasjonen
Siden det ikke enda finnes en pipeline for å bygge appen er man nødt til å selv
kompilere den fra kildekoden. For dette må man installere 
[rust toolchainet](https://rustup.rs/) og kjøre
```bash
cargo build
```

### Kjøre den som root
Om man ønsker at autoforward automatisk oppdaterer /etc/hosts og binde til port
443 må appen kjøre som root. Om man har satt `$KUBECONFIG` i en profil-spesifikk
må man passe gjennom 

### Kjøre appen uten root


## Generer sertifikat for https
Proxyen benytter https for å ligne mest mulig på hvordan ingressene blir registert
i preprod. Dette gjør at når appen binder på port 443 vil flest mulig apper fungere
som normalt.
```bash
./generate_keys.sh
```

## Hvordan fungerer den?

### Trust i Chrome under macOS
Chrome har ingen måte å godkjenne selv-signerte sertifikater on-the-go. For å kunne
benytte proxyen i Chrome må man derfor legge til server.crt i keychain access. Når
det er lagt inn må man markere det som trusted. Sertifikatet er generert til å kun
matche preprod domener.
* Finn server.crt, dobbeltklikk på filen
* I keychain acess finn sertifikatet med label nais.io
* Høyreklikk på sertifikatet og velg get info
* Under trust kan man sette Secure Socket Layer til Always Trust
