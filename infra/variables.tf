variable "aws_region" {
  type    = string
  default = "us-east-1"
}

variable "aws_profile" {
  type    = string
  default = "commma-admin"
}

variable "project" {
  type    = string
  default = "ferrox-subscriber"
}

variable "container_port" {
  type    = number
  default = 9001
}

variable "health_port" {
  type    = number
  default = 9002
}

variable "relay_source_cidr" {
  description = "CIDR allowed to send UDP execution reports to the NLB (the on-prem relay's egress, or a test source)."
  type        = string
}

variable "task_cpu" {
  type    = number
  default = 256
}

variable "task_memory" {
  type    = number
  default = 512
}

variable "image_tag" {
  type    = string
  default = "latest"
}
