{{- define "firelight.name" -}}
{{- .Chart.Name -}}
{{- end -}}

{{- define "firelight.fullname" -}}
{{- if contains .Chart.Name .Release.Name -}}
{{- .Release.Name | trunc 63 | trimSuffix "-" -}}
{{- else -}}
{{- printf "%s-%s" .Release.Name .Chart.Name | trunc 63 | trimSuffix "-" -}}
{{- end -}}
{{- end -}}

{{- define "firelight.selectorLabels" -}}
app.kubernetes.io/name: {{ include "firelight.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end -}}

{{- define "firelight.labels" -}}
{{ include "firelight.selectorLabels" . }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end -}}

{{/* The image reference: digest, else tag, else v<appVersion>. */}}
{{- define "firelight.image" -}}
{{- if .Values.image.digest -}}
{{- printf "%s@%s" .Values.image.repository .Values.image.digest -}}
{{- else if .Values.image.tag -}}
{{- printf "%s:%s" .Values.image.repository .Values.image.tag -}}
{{- else if eq (toString .Chart.AppVersion) "0.0.0" -}}
{{- fail "image.tag or image.digest is required for a source checkout" -}}
{{- else -}}
{{- printf "%s:v%s" .Values.image.repository .Chart.AppVersion -}}
{{- end -}}
{{- end -}}

{{/* The rendered firelight.toml; chart-set keys win over config. */}}
{{- define "firelight.config" -}}
{{- if not .Values.publicUrl -}}
{{- fail "publicUrl is required: the URL people open, e.g. https://firelight.acme.example" -}}
{{- end -}}
{{- $set := dict "server" (dict "listen" "0.0.0.0:8080" "public_url" .Values.publicUrl) "data" (dict "dir" "/var/lib/firelight") -}}
{{- toToml (mergeOverwrite (deepCopy .Values.config) $set) -}}
{{- end -}}
