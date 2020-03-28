#!/bin/bash

mkdir .keys/
openssl genrsa -aes256 -passout pass:insecure -out .keys/root.key 4096
openssl req -x509 -passin pass:insecure -new -nodes -subj "/C=NO/ST=Oslo/L=Oslo/O=NAV/OU=Utvikling/CN=autoforward.nais.io" -key .keys/root.key -sha256 -days 3650 -out .keys/root.pem

openssl req -new -sha256 -nodes -out .keys/server.crt -newkey rsa:4096 -keyout .keys/server.key -config server.crt.cnf
openssl x509 -req -passin pass:insecure -in .keys/server.crt -CA .keys/root.pem -CAkey .keys/root.key -CAcreateserial -out .keys/server.crt -days 3650 -sha256 -extfile server.ext
