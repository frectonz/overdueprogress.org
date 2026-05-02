{
  services.caddy = {
    enable = true;
    email = "fraol0912@gmail.com";
    virtualHosts."overdueprogress.org".extraConfig = ''
      header {
        Strict-Transport-Security "max-age=31536000; includeSubDomains"
        X-Content-Type-Options "nosniff"
        X-Frame-Options "DENY"
        Referrer-Policy "strict-origin-when-cross-origin"
        Permissions-Policy "camera=(), microphone=(), geolocation=(), interest-cohort=()"
        Content-Security-Policy "default-src 'self'; script-src 'self' 'unsafe-inline' https://challenges.cloudflare.com https://static.cloudflareinsights.com; style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; font-src 'self' https://fonts.gstatic.com; img-src 'self' data:; connect-src 'self' https://cloudflareinsights.com; frame-src https://challenges.cloudflare.com; frame-ancestors 'none'; base-uri 'self'; form-action 'self'"
      }
      reverse_proxy 127.0.0.1:3000
    '';
    virtualHosts."www.overdueprogress.org".extraConfig = ''
      redir https://overdueprogress.org{uri} permanent
    '';
  };
}
