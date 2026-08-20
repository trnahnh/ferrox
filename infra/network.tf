# Uses the account's default VPC — this is a single decoupled subscriber,
# not a workload that needs network isolation from anything else in the
# account (see docs/SYSTEM_DESIGN.md §13.3).

data "aws_vpc" "default" {
  default = true
}

data "aws_subnets" "default" {
  filter {
    name   = "vpc-id"
    values = [data.aws_vpc.default.id]
  }

  filter {
    name   = "default-for-az"
    values = ["true"]
  }
}

resource "aws_security_group" "nlb" {
  name        = "${var.project}-nlb"
  description = "Inbound UDP execution reports from the relay to the NLB"
  vpc_id      = data.aws_vpc.default.id

  ingress {
    description = "ExecutionReport stream, relay to NLB"
    from_port   = var.container_port
    to_port     = var.container_port
    protocol    = "udp"
    cidr_blocks = [var.relay_source_cidr]
  }

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }
}

resource "aws_security_group" "task" {
  name        = "${var.project}-task"
  description = "Fargate task: UDP data plane from the NLB, TCP health check from the NLB"
  vpc_id      = data.aws_vpc.default.id

  ingress {
    description     = "ExecutionReport stream, NLB to task"
    from_port       = var.container_port
    to_port         = var.container_port
    protocol        = "udp"
    security_groups = [aws_security_group.nlb.id]
  }

  ingress {
    description = "NLB health check (UDP target groups require TCP health checks)"
    from_port   = var.health_port
    to_port     = var.health_port
    protocol    = "tcp"
    cidr_blocks = [data.aws_vpc.default.cidr_block]
  }

  egress {
    from_port   = 0
    to_port     = 0
    protocol    = "-1"
    cidr_blocks = ["0.0.0.0/0"]
  }
}
