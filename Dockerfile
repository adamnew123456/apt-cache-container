FROM docker.io/debian:stable-slim
RUN apt update && apt install -y approx
COPY --chmod=555 approx_host.bin /
COPY --chmod=555 init.sh /
COPY --chmod=644 approx.conf /etc/approx/approx.conf
EXPOSE 80
ENTRYPOINT /init.sh
