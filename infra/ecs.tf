resource "aws_cloudwatch_log_group" "subscriber" {
  name              = "/ecs/${var.project}"
  retention_in_days = 1
}

resource "aws_ecs_cluster" "this" {
  name = var.project
}

resource "aws_ecs_task_definition" "subscriber" {
  family                   = var.project
  requires_compatibilities = ["FARGATE"]
  network_mode             = "awsvpc"
  cpu                      = var.task_cpu
  memory                   = var.task_memory
  execution_role_arn       = aws_iam_role.task_execution.arn
  task_role_arn            = aws_iam_role.task.arn

  container_definitions = jsonencode([
    {
      name      = "subscriber"
      image     = "${aws_ecr_repository.subscriber.repository_url}:${var.image_tag}"
      essential = true
      portMappings = [
        { containerPort = var.container_port, protocol = "udp" },
        { containerPort = var.health_port, protocol = "tcp" },
      ]
      environment = [
        { name = "BIND_ADDR", value = "0.0.0.0:${var.container_port}" },
        { name = "HEALTH_PORT", value = tostring(var.health_port) },
        # MULTICAST_GROUP intentionally unset: the VPC has no multicast
        # domain (docs/SYSTEM_DESIGN.md §13.2), so the task runs in plain
        # unicast mode and receives traffic forwarded by examples/relay.rs.
      ]
      logConfiguration = {
        logDriver = "awslogs"
        options = {
          "awslogs-group"         = aws_cloudwatch_log_group.subscriber.name
          "awslogs-region"        = var.aws_region
          "awslogs-stream-prefix" = "subscriber"
        }
      }
    }
  ])
}

resource "aws_ecs_service" "subscriber" {
  name                   = var.project
  cluster                = aws_ecs_cluster.this.id
  task_definition        = aws_ecs_task_definition.subscriber.arn
  desired_count          = 1
  launch_type            = "FARGATE"
  enable_execute_command = true

  network_configuration {
    subnets          = data.aws_subnets.default.ids
    security_groups  = [aws_security_group.task.id]
    assign_public_ip = true
  }

  load_balancer {
    target_group_arn = aws_lb_target_group.subscriber.arn
    container_name   = "subscriber"
    container_port   = var.container_port
  }

  depends_on = [aws_lb_listener.subscriber]
}
