Infra holds the Terraform for Acme's cloud accounts and the Helm releases for
every cluster. Changes go through a pull request and Argo CD rolls them out.

## What lives here

- `terraform/`: accounts, networks, access.
- `releases/`: one Helm release per app and environment.
