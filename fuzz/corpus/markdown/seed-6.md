---
format: 1
title: Argo CD
summary: Rolls every Helm release in this repo out to its cluster, and shows what is deployed where.
aliases: [argo, gitops, deployments]
tags: [deploy]
links: [argo-production, argo-staging]
---
Argo CD watches `releases/` and keeps every cluster in line with it. Open
[Argo for staging](link:argo-staging) to see what runs there.

## What it is not

It doesn't build images. CI in each app repo does that, see
[deploying an app](entry:acme-app/deploy-an-app) for the whole path.
