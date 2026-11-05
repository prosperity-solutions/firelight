---
format: 1
title: Schemes
summary: Every kind of link that must be refused, and an image.
---
[a](javascript:alert(1)) [b](data:text/html,hi) [c](file:///etc/passwd) [d](relative/page) [ok](https://ok.acme.example) [m](mailto:x@acme.example) [h](#schemes)

![chart](https://img.acme.example/chart.png)

<script>alert(1)</script> is escaped when rendered, not an error.
