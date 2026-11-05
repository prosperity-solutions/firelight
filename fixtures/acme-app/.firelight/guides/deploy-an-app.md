---
format: 1
title: Deploy the app
summary: From a merged pull request to the app running in staging and production.
aliases: [deploy, ship, rollout]
tags: [deploy]
links: [app-staging, acme-infra/argo-staging]
related: [release-train, acme-infra/argocd]
covers: [deploy/**, Dockerfile]
---
1. Merge your pull request. CI builds the image.
2. Watch it reach staging in [Argo CD](link:acme-infra/argo-staging).
3. Check [the app in staging](link:app-staging).
4. It reaches production with the next [release train](entry:release-train).

## Hotfixes

Label the pull request `hotfix`; it ships right after its checks pass.

| step | where |
|---|---|
| build | CI |
| deploy | Argo CD |

- [x] merged
- [ ] verified in staging
