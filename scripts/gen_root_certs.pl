#!/usr/bin/env perl
# Transcribe node's bundled root certificates into src/stdlib/root_certs.pem.
# Input is src/node_root_certs.h from the node source tree of the reference
# release (v26.12.1):
#   perl scripts/gen_root_certs.pl node_root_certs.h > src/stdlib/root_certs.pem
# Each C string literal is unescaped and the certificates are written in the
# header's order, which is the order of `tls.rootCertificates`, one PEM block
# per certificate.
use strict; use warnings;
my $cert = '';
my $count = 0;
while (my $line = <>) {
  next unless $line =~ /^"(.*)",?\s*$/;
  (my $text = $1) =~ s/\\n/\n/g;
  $cert .= $text;
  if ($text =~ /-----END CERTIFICATE-----$/) {
    print "$cert\n";
    $cert = '';
    $count++;
  }
}
die "unterminated certificate\n" if $cert ne '';
die "no certificates\n" unless $count;
