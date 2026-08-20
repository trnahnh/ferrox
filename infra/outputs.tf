output "ecr_repository_url" {
  value = aws_ecr_repository.subscriber.repository_url
}

output "nlb_dns_name" {
  value = aws_lb.subscriber.dns_name
}

output "cluster_name" {
  value = aws_ecs_cluster.this.name
}

output "log_group" {
  value = aws_cloudwatch_log_group.subscriber.name
}
