# TODO

- [ ] AVDECC push updates: connect to the 848's TCP 17221 proxy, enumerate and map the device descriptors, safely register for unsolicited notifications, renew the registration when required, and retain HTTP polling as a recovery fallback.
- [ ] Hardware A/B/C monitor selection and groups: map the AVDECC descriptors for the configured monitor sets and their output assignments. The HTTP compatibility datastore exposes analog output trims but not a safe A/B/C control mapping.
- [ ] Native IPv6 mDNS: enumerate multicast-capable interfaces and send scoped `_avdecc._tcp.local` queries to `ff02::fb`, removing the current Avahi dependency for IPv6-only discovery.
