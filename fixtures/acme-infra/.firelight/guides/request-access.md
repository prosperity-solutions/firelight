---
format: 1
title: Request access rights
summary: Get time-limited admin rights for a cloud account, with a reason.
aliases: [just-in-time access, elevate, admin rights]
tags: [access, cloud]
links: [access-portal]
related: [access-model, acme-app/onboarding]
covers: [terraform/access/**]
spaces: [platform, support]
---
1. Open [the access portal](link:access-portal).
2. Pick the role and give a reason.
3. The rights end on their own after four hours.

## Roles per environment

| Environment | Role | Approver | Portal | Lasts |
|---|---|---|---|---|
| Production | `admin-production-eu-west` | Platform on-call | `access.acme.example/production` | 4 hours |
| Staging | `admin-staging-eu-west` | Any platform engineer | `access.acme.example/staging` | 8 hours |

## What it does not do

It never grants rights to other people. See
[the access model](entry:access-model#what-it-is-not).
